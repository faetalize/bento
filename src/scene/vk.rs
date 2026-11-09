//! The scene's Vulkan renderer: draws the scene's items into buffers that bento's compositing
//! shows like any surface's, shared as dmabufs.
//!
//! It has a device of its own on bento's GPU. Shapes and glyphs are instanced quads read from one
//! storage buffer; glyphs come from an atlas that swash rasterizes into.

use std::collections::HashMap;
use std::ffi::CStr;
use std::os::fd::{FromRawFd as _, OwnedFd};

use anyhow::{anyhow, bail, Context as _};
use ash::vk;
use smithay::backend::allocator::dmabuf::{Dmabuf, DmabufFlags};
use smithay::backend::allocator::{Format as DrmFormat, Fourcc, Modifier};
use smithay::backend::drm::DrmNode;
use smithay::backend::vulkan::version::Version;
use smithay::backend::vulkan::{Instance, PhysicalDevice};
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::{Format, Vector};

use super::Item;

const SHADER: &str = include_str!("scene.wgsl");
const ATLAS_SIZE: u32 = 2048;
/// The most items a frame draws.
const MAX_ITEMS: usize = 16384;
const ITEM_SIZE: usize = 128;
const STAGING_SIZE: usize = 4 * 1024 * 1024;

/// One item as the shader reads it.
#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuItem {
    rect: [f32; 4],
    radii: [f32; 4],
    fill: [f32; 4],
    border_color: [f32; 4],
    clip_rect: [f32; 4],
    clip_radii: [f32; 4],
    uv: [f32; 4],
    params: [f32; 4],
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: u64,
    index: u32,
    glyph: u32,
    /// The size in physical pixels, in quarters.
    size: u32,
    /// Where in a pixel it starts, in quarters.
    subpixel: u8,
    coords: u64,
}

#[derive(Clone, Copy)]
struct Glyph {
    /// Its place in the atlas, in pixels: x, y, width, height.
    at: [u32; 4],
    left: i32,
    top: i32,
}

struct Buffer {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *mut u8,
}

/// An image the scene is drawn into, shared with bento's compositing as a dmabuf.
pub struct Target {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    pub size: (u32, u32),
    pub dmabuf: Dmabuf,
}

pub struct Renderer {
    device: ash::Device,
    queue: vk::Queue,
    queue_family: u32,
    memory_types: vk::PhysicalDeviceMemoryProperties,
    memory_fd: ash::khr::external_memory_fd::Device,
    modifiers: ash::ext::image_drm_format_modifier::Device,
    /// The modifiers the renderer can draw into and bento's compositing can read.
    allowed: Vec<u64>,
    pool: vk::CommandPool,
    commands: vk::CommandBuffer,
    fence: vk::Fence,
    set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    set: vk::DescriptorSet,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    sampler: vk::Sampler,
    items: Buffer,
    staging: Buffer,
    atlas: vk::Image,
    atlas_memory: vk::DeviceMemory,
    atlas_view: vk::ImageView,
    atlas_ready: bool,
    glyphs: HashMap<GlyphKey, Option<Glyph>>,
    /// The atlas's shelves: where the next glyph goes, and how tall this shelf is.
    shelf: [u32; 3],
    uploads: Vec<(usize, [u32; 4])>,
    staged: usize,
    scaler: ScaleContext,
    _instance: Instance,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("scene::vk::Renderer")
    }
}

fn find_memory(
    props: &vk::PhysicalDeviceMemoryProperties,
    bits: u32,
    flags: vk::MemoryPropertyFlags,
) -> Option<u32> {
    (0..props.memory_type_count).find(|&i| {
        bits & (1 << i) != 0
            && props.memory_types[i as usize]
                .property_flags
                .contains(flags)
    })
}

impl Renderer {
    /// A renderer on the GPU with `node`, drawing what `formats`, bento's compositing's, can read.
    pub fn new(node: Option<DrmNode>, formats: &[DrmFormat]) -> anyhow::Result<Self> {
        let instance = Instance::new(Version::VERSION_1_3, None).context("no Vulkan instance")?;
        let phd = PhysicalDevice::enumerate(&instance)?
            .filter(|phd| phd.api_version() >= Version::VERSION_1_3)
            .find(|phd| match node {
                Some(node) => {
                    phd.render_node().ok().flatten() == Some(node)
                        || phd.primary_node().ok().flatten() == Some(node)
                }
                None => true,
            })
            .ok_or_else(|| anyhow!("no Vulkan 1.3 device for {node:?}"))?;
        let ash_instance = instance.handle().clone();

        let allowed: Vec<u64> = phd
            .get_format_modifier_properties(vk::Format::B8G8R8A8_UNORM)
            .map_err(|err| anyhow!("{err:?}"))?
            .into_iter()
            .filter(|m| {
                m.drm_format_modifier_plane_count == 1
                    && m.drm_format_modifier_tiling_features
                        .contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT)
            })
            .map(|m| m.drm_format_modifier)
            .filter(|m| {
                formats
                    .iter()
                    .any(|f| f.code == Fourcc::Argb8888 && u64::from(f.modifier) == *m)
            })
            .collect();
        if allowed.is_empty() {
            bail!("no modifier both Vulkan and bento's compositing take");
        }

        let queue_family =
            unsafe { ash_instance.get_physical_device_queue_family_properties(phd.handle()) }
                .iter()
                .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))
                .ok_or_else(|| anyhow!("no graphics queue"))? as u32;

        let extensions: [&CStr; 4] = [
            ash::khr::external_memory_fd::NAME,
            ash::ext::external_memory_dma_buf::NAME,
            ash::ext::image_drm_format_modifier::NAME,
            ash::ext::queue_family_foreign::NAME,
        ];
        let extension_ptrs: Vec<_> = extensions.iter().map(|e| e.as_ptr()).collect();
        let priorities = [1.0f32];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities)];
        let mut features13 = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(true)
            .synchronization2(true);
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_extension_names(&extension_ptrs)
            .push_next(&mut features13);
        let device = unsafe { ash_instance.create_device(phd.handle(), &device_info, None)? };
        let queue = unsafe { device.get_device_queue(queue_family, 0) };
        let memory_types =
            unsafe { ash_instance.get_physical_device_memory_properties(phd.handle()) };
        let memory_fd = ash::khr::external_memory_fd::Device::new(&ash_instance, &device);
        let modifiers = ash::ext::image_drm_format_modifier::Device::new(&ash_instance, &device);

        unsafe {
            let pool = device.create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )?;
            let commands = device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .command_buffer_count(1),
            )?[0];
            let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;

            let bindings = [
                vk::DescriptorSetLayoutBinding::default()
                    .binding(0)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(1)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
                vk::DescriptorSetLayoutBinding::default()
                    .binding(2)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            ];
            let set_layout = device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                None,
            )?;
            let sizes = [
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::SAMPLED_IMAGE)
                    .descriptor_count(1),
                vk::DescriptorPoolSize::default()
                    .ty(vk::DescriptorType::SAMPLER)
                    .descriptor_count(1),
            ];
            let descriptor_pool = device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(1)
                    .pool_sizes(&sizes),
                None,
            )?;
            let set_layouts = [set_layout];
            let set = device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(descriptor_pool)
                    .set_layouts(&set_layouts),
            )?[0];

            let push = [vk::PushConstantRange::default()
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
                .size(16)];
            let layout = device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default()
                    .set_layouts(&set_layouts)
                    .push_constant_ranges(&push),
                None,
            )?;
            let pipeline = create_pipeline(&device, layout)?;
            let sampler = device.create_sampler(
                &vk::SamplerCreateInfo::default()
                    .mag_filter(vk::Filter::NEAREST)
                    .min_filter(vk::Filter::NEAREST)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                None,
            )?;

            let host =
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let items = create_buffer(
                &device,
                &memory_types,
                (MAX_ITEMS * ITEM_SIZE) as u64,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                host,
            )?;
            let staging = create_buffer(
                &device,
                &memory_types,
                STAGING_SIZE as u64,
                vk::BufferUsageFlags::TRANSFER_SRC,
                host,
            )?;

            let atlas = device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(vk::Format::R8_UNORM)
                    .extent(vk::Extent3D {
                        width: ATLAS_SIZE,
                        height: ATLAS_SIZE,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST),
                None,
            )?;
            let req = device.get_image_memory_requirements(atlas);
            let atlas_memory = device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(
                        find_memory(
                            &memory_types,
                            req.memory_type_bits,
                            vk::MemoryPropertyFlags::DEVICE_LOCAL,
                        )
                        .ok_or_else(|| anyhow!("no memory for the atlas"))?,
                    ),
                None,
            )?;
            device.bind_image_memory(atlas, atlas_memory, 0)?;
            let atlas_view = device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(atlas)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(vk::Format::R8_UNORM)
                    .subresource_range(color_range()),
                None,
            )?;

            let buffer_info = [vk::DescriptorBufferInfo::default()
                .buffer(items.buffer)
                .range(vk::WHOLE_SIZE)];
            let image_info = [vk::DescriptorImageInfo::default()
                .image_view(atlas_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let sampler_info = [vk::DescriptorImageInfo::default().sampler(sampler)];
            device.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(&buffer_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .image_info(&image_info),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(2)
                        .descriptor_type(vk::DescriptorType::SAMPLER)
                        .image_info(&sampler_info),
                ],
                &[],
            );

            Ok(Self {
                device,
                queue,
                queue_family,
                memory_types,
                memory_fd,
                modifiers,
                allowed,
                pool,
                commands,
                fence,
                set_layout,
                descriptor_pool,
                set,
                layout,
                pipeline,
                sampler,
                items,
                staging,
                atlas,
                atlas_memory,
                atlas_view,
                atlas_ready: false,
                glyphs: HashMap::new(),
                shelf: [0, 0, 0],
                uploads: Vec::new(),
                staged: 0,
                scaler: ScaleContext::new(),
                _instance: instance,
            })
        }
    }

    /// An image `size` big, in physical pixels, to draw into.
    pub fn create_target(&self, size: (u32, u32)) -> anyhow::Result<Target> {
        let device = &self.device;
        unsafe {
            let mut list = vk::ImageDrmFormatModifierListCreateInfoEXT::default()
                .drm_format_modifiers(&self.allowed);
            let mut external = vk::ExternalMemoryImageCreateInfo::default()
                .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let image = device.create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(vk::Format::B8G8R8A8_UNORM)
                    .extent(vk::Extent3D {
                        width: size.0,
                        height: size.1,
                        depth: 1,
                    })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                    .usage(
                        vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                    )
                    .push_next(&mut list)
                    .push_next(&mut external),
                None,
            )?;
            let req = device.get_image_memory_requirements(image);
            let mut export = vk::ExportMemoryAllocateInfo::default()
                .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
            let memory = device.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(
                        find_memory(
                            &self.memory_types,
                            req.memory_type_bits,
                            vk::MemoryPropertyFlags::DEVICE_LOCAL,
                        )
                        .ok_or_else(|| anyhow!("no memory for a target"))?,
                    )
                    .push_next(&mut export)
                    .push_next(&mut dedicated),
                None,
            )?;
            device.bind_image_memory(image, memory, 0)?;

            let mut props = vk::ImageDrmFormatModifierPropertiesEXT::default();
            self.modifiers
                .get_image_drm_format_modifier_properties(image, &mut props)?;
            let layout = device.get_image_subresource_layout(
                image,
                vk::ImageSubresource::default()
                    .aspect_mask(vk::ImageAspectFlags::MEMORY_PLANE_0_EXT),
            );
            let fd = self.memory_fd.get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
            )?;
            let fd = OwnedFd::from_raw_fd(fd);
            let mut builder = Dmabuf::builder(
                (size.0 as i32, size.1 as i32),
                Fourcc::Argb8888,
                Modifier::from(props.drm_format_modifier),
                DmabufFlags::empty(),
            );
            builder.add_plane(fd, layout.offset as u32, layout.row_pitch as u32);
            let dmabuf = builder
                .build()
                .ok_or_else(|| anyhow!("couldn't share a target"))?;

            let view = device.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(vk::Format::B8G8R8A8_UNORM)
                    .subresource_range(color_range()),
                None,
            )?;
            Ok(Target {
                image,
                memory,
                view,
                size,
                dmabuf,
            })
        }
    }

    pub fn destroy_target(&self, target: Target) {
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_image_view(target.view, None);
            self.device.destroy_image(target.image, None);
            self.device.free_memory(target.memory, None);
        }
    }

    /// Where a glyph is in the atlas, rasterizing it if it isn't yet, or `None` for one with
    /// nothing to draw.
    fn glyph(&mut self, key: GlyphKey, font: &parley::FontData, coords: &[i16]) -> Option<Glyph> {
        if let Some(glyph) = self.glyphs.get(&key) {
            return *glyph;
        }
        let glyph = (|| {
            let font_ref = swash::FontRef::from_index(font.data.as_ref(), font.index as usize)?;
            let mut scaler = self
                .scaler
                .builder(font_ref)
                .size(key.size as f32 / 4.)
                .hint(true)
                .normalized_coords(coords)
                .build();
            let image = Render::new(&[Source::Outline])
                .format(Format::Alpha)
                .offset(Vector::new(f32::from(key.subpixel) / 4., 0.))
                .render(&mut scaler, key.glyph as u16)?;
            let (w, h) = (image.placement.width, image.placement.height);
            if w == 0 || h == 0 {
                return None;
            }
            // The next place on the shelf, or a new shelf.
            let [mut x, mut y, mut shelf_h] = self.shelf;
            if x + w + 1 > ATLAS_SIZE {
                x = 0;
                y += shelf_h + 1;
                shelf_h = 0;
            }
            if y + h + 1 > ATLAS_SIZE || self.staged + image.data.len() > STAGING_SIZE {
                // Full: it's drawn again from scratch next time.
                return None;
            }
            self.shelf = [x + w + 1, y, shelf_h.max(h)];
            unsafe {
                std::ptr::copy_nonoverlapping(
                    image.data.as_ptr(),
                    self.staging.mapped.add(self.staged),
                    image.data.len(),
                );
            }
            self.uploads.push((self.staged, [x, y, w, h]));
            self.staged += image.data.len();
            Some(Glyph {
                at: [x, y, w, h],
                left: image.placement.left,
                top: image.placement.top,
            })
        })();
        self.glyphs.insert(key, glyph);
        glyph
    }

    /// Draws `items` into `target`, at `scale`, and waits until it's drawn.
    pub fn render(&mut self, target: &Target, items: &[Item], scale: f32) -> anyhow::Result<()> {
        if self.glyphs.len() > 8192 || self.shelf[1] + 64 > ATLAS_SIZE {
            self.glyphs.clear();
            self.shelf = [0, 0, 0];
        }

        let mut gpu: Vec<GpuItem> = Vec::with_capacity(items.len());
        let no_clip = ([0., 0., -1., -1.], [0.; 4]);
        let premul = |c: [f32; 4]| [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]];
        for item in items {
            match item {
                Item::Shape {
                    rect,
                    radii,
                    material,
                    fill,
                    border,
                    opacity,
                    clip,
                } => {
                    // A material's tint is drawn over what bento draws of the material.
                    let _ = material;
                    let (clip_rect, clip_radii) = clip.map_or(no_clip, |c| (c.rect, c.radii));
                    gpu.push(GpuItem {
                        rect: *rect,
                        radii: *radii,
                        fill: premul(*fill),
                        border_color: premul(border.1),
                        clip_rect,
                        clip_radii,
                        uv: [0.; 4],
                        params: [0., border.0, *opacity, 0.],
                    });
                }
                Item::Text {
                    origin,
                    laid,
                    color,
                    opacity,
                    clip,
                } => {
                    let (clip_rect, clip_radii) = clip.map_or(no_clip, |c| (c.rect, c.radii));
                    for run in &laid.runs {
                        let coords = {
                            use std::hash::{Hash, Hasher};
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            run.coords.hash(&mut hasher);
                            hasher.finish()
                        };
                        for &(glyph, x, y) in &run.glyphs {
                            let px = (origin[0] + x) * scale;
                            let py = ((origin[1] + y) * scale).round();
                            let subpixel = ((px.fract() * 4.).floor() as u8).min(3);
                            let key = GlyphKey {
                                font: run.font.data.id(),
                                index: run.font.index,
                                glyph,
                                size: (run.size * scale * 4.).round() as u32,
                                subpixel,
                                coords,
                            };
                            let Some(g) = self.glyph(key, &run.font, &run.coords) else {
                                continue;
                            };
                            let gx = px.floor() + g.left as f32;
                            let gy = py - g.top as f32;
                            let atlas = ATLAS_SIZE as f32;
                            gpu.push(GpuItem {
                                rect: [
                                    gx / scale,
                                    gy / scale,
                                    g.at[2] as f32 / scale,
                                    g.at[3] as f32 / scale,
                                ],
                                radii: [0.; 4],
                                fill: premul(*color),
                                border_color: [0.; 4],
                                clip_rect,
                                clip_radii,
                                uv: [
                                    g.at[0] as f32 / atlas,
                                    g.at[1] as f32 / atlas,
                                    (g.at[0] + g.at[2]) as f32 / atlas,
                                    (g.at[1] + g.at[3]) as f32 / atlas,
                                ],
                                params: [1., 0., *opacity, 0.],
                            });
                        }
                    }
                }
            }
        }
        gpu.truncate(MAX_ITEMS);
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytemuck::cast_slice::<GpuItem, u8>(&gpu).as_ptr(),
                self.items.mapped,
                gpu.len() * ITEM_SIZE,
            );
        }

        let device = &self.device;
        let cmd = self.commands;
        unsafe {
            device.reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())?;
            device.begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;

            // New glyphs into the atlas.
            if !self.uploads.is_empty() || !self.atlas_ready {
                let old = if self.atlas_ready {
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
                } else {
                    vk::ImageLayout::UNDEFINED
                };
                barrier(
                    device,
                    cmd,
                    self.atlas,
                    old,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    None,
                );
                let regions: Vec<_> = self
                    .uploads
                    .iter()
                    .map(|(offset, [x, y, w, h])| {
                        vk::BufferImageCopy::default()
                            .buffer_offset(*offset as u64)
                            .image_subresource(
                                vk::ImageSubresourceLayers::default()
                                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                                    .layer_count(1),
                            )
                            .image_offset(vk::Offset3D {
                                x: *x as i32,
                                y: *y as i32,
                                z: 0,
                            })
                            .image_extent(vk::Extent3D {
                                width: *w,
                                height: *h,
                                depth: 1,
                            })
                    })
                    .collect();
                if !regions.is_empty() {
                    device.cmd_copy_buffer_to_image(
                        cmd,
                        self.staging.buffer,
                        self.atlas,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &regions,
                    );
                }
                barrier(
                    device,
                    cmd,
                    self.atlas,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    None,
                );
                self.atlas_ready = true;
            }

            // Taken from bento's compositing, which may still be reading the last frame.
            barrier(
                device,
                cmd,
                target.image,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                Some((vk::QUEUE_FAMILY_FOREIGN_EXT, self.queue_family)),
            );
            let attachments = [vk::RenderingAttachmentInfo::default()
                .image_view(target.view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: [0., 0., 0., 0.],
                    },
                })];
            let area = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: target.size.0,
                    height: target.size.1,
                },
            };
            device.cmd_begin_rendering(
                cmd,
                &vk::RenderingInfo::default()
                    .render_area(area)
                    .layer_count(1)
                    .color_attachments(&attachments),
            );
            device.cmd_set_viewport(
                cmd,
                0,
                &[vk::Viewport {
                    x: 0.,
                    y: 0.,
                    width: target.size.0 as f32,
                    height: target.size.1 as f32,
                    min_depth: 0.,
                    max_depth: 1.,
                }],
            );
            device.cmd_set_scissor(cmd, 0, &[area]);
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                0,
                &[self.set],
                &[],
            );
            let frame = [
                target.size.0 as f32 / scale,
                target.size.1 as f32 / scale,
                scale,
                0.,
            ];
            device.cmd_push_constants(
                cmd,
                self.layout,
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                bytemuck::cast_slice(&frame),
            );
            if !gpu.is_empty() {
                device.cmd_draw(cmd, 6, gpu.len() as u32, 0, 0);
            }
            device.cmd_end_rendering(cmd);
            // Handed to bento's compositing.
            barrier(
                device,
                cmd,
                target.image,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::ImageLayout::GENERAL,
                Some((self.queue_family, vk::QUEUE_FAMILY_FOREIGN_EXT)),
            );
            device.end_command_buffer(cmd)?;

            let buffers = [cmd];
            device.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default().command_buffers(&buffers)],
                self.fence,
            )?;
            device.wait_for_fences(&[self.fence], true, u64::MAX)?;
            device.reset_fences(&[self.fence])?;
        }
        self.uploads.clear();
        self.staged = 0;
        Ok(())
    }
}

#[cfg(test)]
impl Renderer {
    /// What `target` holds, as rows of BGRA pixels.
    fn read_back(&self, target: &Target) -> anyhow::Result<Vec<u8>> {
        let (w, h) = target.size;
        let size = u64::from(w * h * 4);
        let device = &self.device;
        unsafe {
            let buffer = create_buffer(
                device,
                &self.memory_types,
                size,
                vk::BufferUsageFlags::TRANSFER_DST,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            )?;
            let cmd = self.commands;
            device.reset_command_buffer(cmd, vk::CommandBufferResetFlags::empty())?;
            device.begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default())?;
            barrier(
                device,
                cmd,
                target.image,
                vk::ImageLayout::GENERAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                Some((vk::QUEUE_FAMILY_FOREIGN_EXT, self.queue_family)),
            );
            device.cmd_copy_image_to_buffer(
                cmd,
                target.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer.buffer,
                &[vk::BufferImageCopy::default()
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_extent(vk::Extent3D {
                        width: w,
                        height: h,
                        depth: 1,
                    })],
            );
            device.end_command_buffer(cmd)?;
            let buffers = [cmd];
            device.queue_submit(
                self.queue,
                &[vk::SubmitInfo::default().command_buffers(&buffers)],
                self.fence,
            )?;
            device.wait_for_fences(&[self.fence], true, u64::MAX)?;
            device.reset_fences(&[self.fence])?;
            let pixels = std::slice::from_raw_parts(buffer.mapped, size as usize).to_vec();
            device.destroy_buffer(buffer.buffer, None);
            device.free_memory(buffer.memory, None);
            Ok(pixels)
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            let d = &self.device;
            let _ = d.device_wait_idle();
            d.destroy_pipeline(self.pipeline, None);
            d.destroy_pipeline_layout(self.layout, None);
            d.destroy_descriptor_pool(self.descriptor_pool, None);
            d.destroy_descriptor_set_layout(self.set_layout, None);
            d.destroy_sampler(self.sampler, None);
            d.destroy_image_view(self.atlas_view, None);
            d.destroy_image(self.atlas, None);
            d.free_memory(self.atlas_memory, None);
            for buffer in [&self.items, &self.staging] {
                d.destroy_buffer(buffer.buffer, None);
                d.free_memory(buffer.memory, None);
            }
            d.destroy_fence(self.fence, None);
            d.destroy_command_pool(self.pool, None);
            d.destroy_device(None);
        }
    }
}

fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1)
}

unsafe fn barrier(
    device: &ash::Device,
    cmd: vk::CommandBuffer,
    image: vk::Image,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
    families: Option<(u32, u32)>,
) {
    let (src, dst) = families.unwrap_or((vk::QUEUE_FAMILY_IGNORED, vk::QUEUE_FAMILY_IGNORED));
    let barriers = [vk::ImageMemoryBarrier2::default()
        .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
        .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
        .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
        .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)
        .old_layout(old)
        .new_layout(new)
        .src_queue_family_index(src)
        .dst_queue_family_index(dst)
        .image(image)
        .subresource_range(color_range())];
    unsafe {
        device.cmd_pipeline_barrier2(
            cmd,
            &vk::DependencyInfo::default().image_memory_barriers(&barriers),
        );
    }
}

unsafe fn create_buffer(
    device: &ash::Device,
    memory_types: &vk::PhysicalDeviceMemoryProperties,
    size: u64,
    usage: vk::BufferUsageFlags,
    flags: vk::MemoryPropertyFlags,
) -> anyhow::Result<Buffer> {
    unsafe {
        let buffer = device.create_buffer(
            &vk::BufferCreateInfo::default().size(size).usage(usage),
            None,
        )?;
        let req = device.get_buffer_memory_requirements(buffer);
        let memory = device.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(
                    find_memory(memory_types, req.memory_type_bits, flags)
                        .ok_or_else(|| anyhow!("no memory for a buffer"))?,
                ),
            None,
        )?;
        device.bind_buffer_memory(buffer, memory, 0)?;
        let mapped = device.map_memory(memory, 0, size, vk::MemoryMapFlags::empty())? as *mut u8;
        Ok(Buffer {
            buffer,
            memory,
            mapped,
        })
    }
}

/// The scene's shader, as SPIR-V for one of its entry points.
fn spirv(stage: naga::ShaderStage, entry: &str) -> anyhow::Result<Vec<u32>> {
    let module = naga::front::wgsl::parse_str(SHADER).map_err(|err| anyhow!("{err}"))?;
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::IMMEDIATES,
    )
    .validate(&module)
    .map_err(|err| anyhow!("{err:?}"))?;
    let options = naga::back::spv::Options::default();
    let pipeline = naga::back::spv::PipelineOptions {
        shader_stage: stage,
        entry_point: String::from(entry),
    };
    naga::back::spv::write_vec(&module, &info, &options, Some(&pipeline))
        .map_err(|err| anyhow!("{err}"))
}

unsafe fn create_pipeline(
    device: &ash::Device,
    layout: vk::PipelineLayout,
) -> anyhow::Result<vk::Pipeline> {
    unsafe {
        let vs = spirv(naga::ShaderStage::Vertex, "vs")?;
        let fs = spirv(naga::ShaderStage::Fragment, "fs")?;
        let vs =
            device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&vs), None)?;
        let fs =
            device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&fs), None)?;
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vs)
                .name(c"vs"),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fs)
                .name(c"fs"),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let blend = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend);
        let dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic);
        let formats = [vk::Format::B8G8R8A8_UNORM];
        let mut rendering =
            vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&formats);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(layout)
            .push_next(&mut rendering);
        let pipeline = device
            .create_graphics_pipelines(vk::PipelineCache::null(), &[info], None)
            .map_err(|(_, err)| err)?[0];
        device.destroy_shader_module(vs, None);
        device.destroy_shader_module(fs, None);
        Ok(pipeline)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_compiles() {
        spirv(naga::ShaderStage::Vertex, "vs").unwrap();
        spirv(naga::ShaderStage::Fragment, "fs").unwrap();
    }

    /// Draws a bar along the top offscreen, on the machine's GPU, and reads it back: it has to
    /// be along the top. It needs a GPU, so it's run by hand.
    #[test]
    #[ignore]
    fn the_top_is_at_the_top() {
        let formats = [DrmFormat {
            code: Fourcc::Argb8888,
            modifier: Modifier::Linear,
        }];
        let mut renderer = Renderer::new(None, &formats).unwrap();
        let target = renderer.create_target((64, 64)).unwrap();
        let bar = Item::Shape {
            rect: [0., 0., 64., 16.],
            radii: [0.; 4],
            material: super::super::Material::None,
            fill: [1., 0., 0., 1.],
            border: (0., [0.; 4]),
            opacity: 1.,
            clip: None,
        };
        renderer.render(&target, &[bar], 1.).unwrap();

        let pixels = renderer.read_back(&target).unwrap();
        // BGRA: alpha is the fourth byte.
        let alpha = |row: usize| pixels[(row * 64 + 32) * 4 + 3];
        let alpha = (alpha(4), alpha(60));
        assert_eq!(alpha, (255, 0), "(top, bottom)");
        renderer.destroy_target(target);
    }

    /// Draws bento's own shell offscreen on the machine's GPU. It needs one, so it's run by hand.
    #[test]
    #[ignore]
    fn it_draws_the_shell() {
        let shell =
            super::super::script::Shell::load(&[(String::from("DP-1"), (1920., 1080.))]).unwrap();
        let items = {
            let mut scene = shell.scene.borrow_mut();
            scene.lay_out();
            scene.advance(1., Some(1.));
            scene.draw("DP-1")
        };
        let modifiers = [Modifier::Linear, Modifier::Invalid]
            .into_iter()
            .chain((0..64).map(|_| Modifier::Invalid));
        let mut formats: Vec<DrmFormat> = modifiers
            .map(|modifier| DrmFormat {
                code: Fourcc::Argb8888,
                modifier,
            })
            .collect();
        // Anything Vulkan offers, as bento's compositing would on this GPU.
        let instance = Instance::new(Version::VERSION_1_3, None).unwrap();
        for phd in PhysicalDevice::enumerate(&instance).unwrap() {
            for m in phd
                .get_format_modifier_properties(vk::Format::B8G8R8A8_UNORM)
                .unwrap_or_default()
            {
                formats.push(DrmFormat {
                    code: Fourcc::Argb8888,
                    modifier: Modifier::from(m.drm_format_modifier),
                });
            }
        }
        let mut renderer = Renderer::new(None, &formats).unwrap();
        let target = renderer.create_target((1920, 1080)).unwrap();
        renderer.render(&target, &items, 1.).unwrap();
        renderer.render(&target, &items, 1.).unwrap();
        renderer.destroy_target(target);
    }
}
