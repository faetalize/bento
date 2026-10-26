bento's documentation is the Markdown in `docs/wiki/`, read on GitHub.
There's no documentation site, and nothing publishes the pages to a GitHub wiki: the pages link to each other with relative links, which GitHub's Markdown preview follows.

`.github/workflows/ci.yml` still has niri's `publish-docs` and `publish-wiki` jobs, but they only run in the upstream niri repository.

## Building the docs locally

The [mkdocs](https://www.mkdocs.org/) setup from niri is still in `docs/`: `mkdocs.yaml`, the Python project in `pyproject.toml` and `uv.lock`, and the hooks in `docs/hooks/`.
Nothing publishes the result, but you can use it to browse the docs locally, and it checks links more strictly than GitHub (see [Links](#links)).

To set it up and run it, it is recommended to use [uv](https://docs.astral.sh/uv/).

### Serving the docs locally with uv

In the `docs/` subdirectory:

- `uv sync`
- `uv run mkdocs serve`

The docs should now be available at the address that mkdocs prints.

Changes made to the documentation while the development server is running will cause an automatic page refresh in the browser.

### Hooks

The hooks in `docs/hooks/` adapt the Markdown for mkdocs:

- `shortcodes.py` turns version annotations like <sup>Since: 25.05</sup> into badges that link to that niri release.
- `remove-must-fail.py` removes the `must-fail` annotation from code blocks (see [Snippets](#snippets)).

## Elements

Elements such as links, admonitions, images, and snippets should work as expected both in GitHub's Markdown preview and in mkdocs.

### Links

Links should in all cases be relative (e.g. `./FAQ.md`), unless it's an external one.
Links should have anchors if they are meant to lead the user to a specific section on a page (e.g. `./Getting-Started.md#nvidia`).

> [!TIP]
> GitHub doesn't check links, but mkdocs will terminate if relative links lead to non-existing documents or non-existing anchors.
> This means that `mkdocs build` and `mkdocs serve` will fail on a broken link, which makes them a quick way to check.

### Admonitions

> [!IMPORTANT]
> This is an important distinction from other `mkdocs`-based documentation you might have encountered.

Admonitions, or alerts should be written [the way GitHub defines them](https://docs.github.com/en/get-started/writing-on-github/getting-started-with-writing-and-formatting-on-github/basic-writing-and-formatting-syntax#alerts).
The above admonition is written like this:

```
> [!IMPORTANT]
> This is an important distinction from other `mkdocs`-based documentation you might have encountered.
```

### Images

Images should have relative links to resources in `docs/wiki/img/`, and should contain sensible alt-text.

### Videos

For compatibility with both mkdocs and GitHub, videos need to be wrapped in a `<video>` tag and have the video link again as fallback text, padded with blank lines.

```html
<video controls src="https://github.com/user-attachments/assets/379a5d1f-acdb-4c11-b36c-e85fd91f0995">

https://github.com/user-attachments/assets/379a5d1f-acdb-4c11-b36c-e85fd91f0995

</video>
```

### Snippets

Configuration and code snippets in general should be annotated with a language.

If the language used in the snippet is KDL, open the code block like this:

```md
```kdl
```

A test in `niri-config/tests/wiki-parses.rs` parses every `kdl` code block in the wiki as a config, so they need to be valid.
If a snippet is meant to fail, add `must-fail` after the language, separated by a comma, and the test will check that it fails instead.
