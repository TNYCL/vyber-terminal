use std::{collections::HashMap, ffi::OsString};

pub fn zsh_integration(
    env: &mut HashMap<String, String>,
    original_zdotdir: Option<OsString>,
) -> anyhow::Result<tempfile::TempDir> {
    let directory = tempfile::Builder::new().prefix("vyber-zsh-").tempdir()?;
    std::fs::write(
        directory.path().join(".zshenv"),
        include_str!("../assets/shell-integration/zsh.zsh"),
    )?;
    for name in [".zprofile", ".zshrc", ".zlogin"] {
        let finish = match name {
            ".zshrc" => {
                "if [[ $HISTFILE == $__vyber_integration_dir/.zsh_history ]]; then\n    HISTFILE=$__vyber_user_zdotdir/.zsh_history\nfi\n__vyber_install_hooks\nif [[ -o login ]]; then\n    ZDOTDIR=$__vyber_integration_dir\nfi\n"
            }
            ".zlogin" => "__vyber_install_hooks\n",
            _ => "ZDOTDIR=$__vyber_integration_dir\n",
        };
        std::fs::write(
            directory.path().join(name),
            format!(
                "__vyber_restore_zdotdir\n[[ -r ${{ZDOTDIR:-$HOME}}/{name} ]] && builtin source \"${{ZDOTDIR:-$HOME}}/{name}\"\n__vyber_remember_zdotdir\n{finish}"
            ),
        )?;
    }
    env.insert(
        "ZDOTDIR".into(),
        directory.path().to_string_lossy().into_owned(),
    );
    if let Some(original) = original_zdotdir {
        env.insert(
            "VYBER_ORIGINAL_ZDOTDIR".into(),
            original.to_string_lossy().into_owned(),
        );
    }
    Ok(directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty::OscParser;

    #[test]
    fn zsh_preserves_profiles_and_reports_directory_changes() -> anyhow::Result<()> {
        let profiles = tempfile::tempdir()?;
        let root = tempfile::tempdir()?;
        let root_path = root.path().canonicalize()?;
        let target = root_path.join("Çalışma alanı % #");
        std::fs::create_dir(&target)?;
        for name in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
            std::fs::write(
                profiles.path().join(name),
                format!("VYBER_STARTUP_ORDER+=\"{name} \"\n"),
            )?;
        }
        std::fs::write(
            profiles.path().join(".zshrc"),
            "VYBER_STARTUP_ORDER+=\".zshrc \"\ntypeset VYBER_PROFILE_VALUE=preserved\nfunction user_prompt_hook() { :; }\nprecmd_functions=(user_prompt_hook)\n",
        )?;
        let mut env = HashMap::new();
        let _integration = zsh_integration(&mut env, Some(profiles.path().as_os_str().to_owned()))?;
        let output = std::process::Command::new("/bin/zsh")
            .args([
                "-l",
                "-i",
                "-c",
                r#"
                [[ $VYBER_STARTUP_ORDER == '.zshenv .zprofile .zshrc .zlogin ' ]] || exit 10
                [[ $ZDOTDIR == $VYBER_TEST_PROFILES ]] || exit 11
                [[ $HISTFILE != */vyber-zsh-*/.zsh_history ]] || exit 12
                [[ ${precmd_functions[(Ie)user_prompt_hook]} -gt 0 ]] || exit 13
                [[ ${#${(M)precmd_functions:#__vyber_report_cwd}} == 1 ]] || exit 14
                [[ $VYBER_PROFILE_VALUE == preserved ]] || exit 16
                cd -- "$VYBER_TEST_TARGET" || exit 15
                print -r -- 'VYBER_TEST_DONE'
            "#,
            ])
            .current_dir(&root_path)
            .envs(env)
            .env("VYBER_TEST_PROFILES", profiles.path())
            .env("VYBER_TEST_TARGET", &target)
            .env("TERM_PROGRAM", "Vyber")
            .env("TERM", "xterm-256color")
            .output()?;
        assert!(
            output.status.success(),
            "zsh failed: {:?}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
        let messages = OscParser::default().feed(&output.stdout);
        let directories = messages
            .iter()
            .filter_map(|message| {
                url::Url::parse(message.strip_prefix("7;")?)
                    .ok()?
                    .to_file_path()
                    .ok()
            })
            .collect::<Vec<_>>();
        assert!(directories.contains(&root_path), "{messages:?}");
        assert!(directories.contains(&target), "{messages:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("VYBER_TEST_DONE"));
        Ok(())
    }
}
