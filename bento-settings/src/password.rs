//! Changing the user's own password the way a terminal does: through `passwd`, which asks PAM.
//! That checks the current password, applies the system's password rules, and lets PAM modules
//! like gnome-keyring's follow along, with no admin rights needed.

use gtk::gio;

pub async fn change(current: &str, new: &str) -> Result<(), String> {
    run(&["passwd"], current, new).await
}

async fn run(argv: &[&str], current: &str, new: &str) -> Result<(), String> {
    // passwd asks for the current password, then the new one twice. PAM reads the answers from
    // stdin when it isn't a terminal; they never appear in the command line.
    let input = format!("{current}\n{new}\n{new}\n");
    let process = gio::Subprocess::newv(
        &argv.iter().map(std::ffi::OsStr::new).collect::<Vec<_>>(),
        gio::SubprocessFlags::STDIN_PIPE
            | gio::SubprocessFlags::STDOUT_PIPE
            | gio::SubprocessFlags::STDERR_MERGE,
    )
    .map_err(|err| err.message().to_owned())?;
    let (output, _) = process
        .communicate_utf8_future(Some(input))
        .await
        .map_err(|err| err.message().to_owned())?;

    if process.is_successful() {
        Ok(())
    } else {
        Err(message(output.as_deref().unwrap_or_default()))
    }
}

/// Picks what went wrong out of passwd's output, which mixes its prompts in with its messages.
fn message(output: &str) -> String {
    const PROMPTS: &[&str] = &[
        "Current password:",
        "New password:",
        "Retype new password:",
        "Retype password:",
        "Password:",
    ];
    let lines: Vec<String> = output
        .lines()
        .map(|line| {
            let mut line = line.trim();
            while let Some(rest) = PROMPTS.iter().find_map(|prompt| line.strip_prefix(prompt)) {
                line = rest.trim();
            }
            line.trim_start_matches("passwd:").trim().to_owned()
        })
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("Changing password for")
                && line != "password unchanged"
        })
        .collect();

    // The password rules explain themselves; other failures are mostly a wrong current password.
    if let Some(line) = lines.iter().find(|line| line.starts_with("BAD PASSWORD")) {
        return line.trim_start_matches("BAD PASSWORD:").trim().to_owned();
    }
    match lines.first() {
        Some(line) if line == "Authentication token manipulation error" => {
            String::from("The current password is wrong")
        }
        Some(line) => line.clone(),
        None => String::from("passwd failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stands in for passwd: wants "old" as the current password and the new one twice.
    const FAKE: &str = r#"printf 'Current password: '; read a; printf 'New password: '; read b; printf 'Retype new password: '; read c
if [ "$a" != old ]; then echo 'passwd: Authentication token manipulation error'; echo 'passwd: password unchanged'; exit 1; fi
if [ ${#b} -lt 8 ]; then echo 'BAD PASSWORD: The password is shorter than 8 characters'; exit 1; fi
[ "$b" = "$c" ]"#;

    fn change(current: &str, new: &str) -> Result<(), String> {
        // Tests run in parallel, so each needs a main context of its own.
        let context = gtk::glib::MainContext::new();
        context
            .with_thread_default(|| context.block_on(run(&["sh", "-c", FAKE], current, new)))
            .unwrap()
    }

    #[test]
    fn feeds_the_prompts() {
        assert_eq!(change("old", "a long new one"), Ok(()));
    }

    #[test]
    fn explains_failures() {
        assert_eq!(
            change("wrong", "a long new one"),
            Err(String::from("The current password is wrong"))
        );
        assert_eq!(
            change("old", "short"),
            Err(String::from("The password is shorter than 8 characters"))
        );
    }
}
