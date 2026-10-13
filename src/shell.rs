#[cfg(unix)]
use std::{collections::HashMap, ffi::OsString};

/// Prefer native OpenSSH in ConPTY-backed Git Bash sessions. The MSYS2 SSH
/// client can exit with `poll: No error` on a console resize (Git for Windows
/// #6413). Install the function at the first prompt, after login profiles,
/// while leaving user aliases/functions and the inherited prompt hook intact.
pub fn windows_bash_prompt(inherited: &str, native_ssh: Option<&std::path::Path>) -> String {
    let ssh = native_ssh.map_or_else(String::new, |path| {
        let path = path
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "'\\''");
        format!(
            "if ! declare -F ssh >/dev/null && ! alias ssh >/dev/null 2>&1; then\n\
             function ssh {{ MSYS2_ARG_CONV_EXCL='*' command '{path}' \"$@\"; }}\n\
             fi\n"
        )
    });
    format!(
        "{inherited}\n__vyber_status=$?; {ssh}\
         if [[ ${{__vyber_prompt_mark:-0}} == 0 ]]; then\n\
         __vyber_prompt_mark=1; __vyber_prompt_title=__VYBER_CWD__\n\
         else\n\
         __vyber_prompt_mark=0; __vyber_prompt_title=__VYBER_CWD2__\n\
         fi\n\
         printf '\\033]0;%s%s\\007' \"$__vyber_prompt_title\" \"$(pwd -W)\"; (exit \"$__vyber_status\")"
    )
}

#[cfg(unix)]
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::pty::OscParser;

    #[cfg(unix)]
    #[test]
    fn windows_bash_ssh_hook_preserves_arguments_overrides_and_exit_status() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir()?;
        let ssh = dir.path().join("native ssh's client");
        std::fs::write(
            &ssh,
            "#!/bin/sh\ntest \"$MSYS2_ARG_CONV_EXCL\" = '*' || exit 91\nprintf '<%s>\\n' \"$@\"\nexit 23\n",
        )?;
        std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755))?;
        let hook = windows_bash_prompt("printf 'INHERITED\\n'; (exit 17)", Some(&ssh));
        for (setup, invocation, expected) in [
            (
                "",
                "ssh -t 'user@host' '/usr/bin/program' 'argument with spaces'; test $? = 23 && test \"$MSYS2_ARG_CONV_EXCL\" = sentinel",
                "<-t>\n<user@host>\n</usr/bin/program>\n<argument with spaces>",
            ),
            (
                "ssh() { printf 'USER_FUNCTION\\n'; }",
                "ssh",
                "USER_FUNCTION",
            ),
            (
                "alias ssh=\"printf 'USER_ALIAS\\\\n'\"",
                "eval ssh",
                "USER_ALIAS",
            ),
        ] {
            let output = std::process::Command::new("bash")
                .args(["--noprofile", "--norc", "-c",
                    "MSYS2_ARG_CONV_EXCL=sentinel; shopt -s expand_aliases; pwd() { builtin pwd; }; eval \"$1\"; eval \"$2\"; test $? = 17 || exit 90; eval \"$2\"; eval \"$3\"",
                    "vyber-test", setup, &hook, invocation])
                .output()?;
            let text = String::from_utf8_lossy(&output.stdout);
            assert!(output.status.success(), "{setup}: {output:?}");
            assert!(
                text.contains("INHERITED") && text.contains(expected),
                "{text}"
            );
            let markers = OscParser::default().feed(&output.stdout);
            assert_eq!(markers.len(), 2, "{markers:?}");
            assert!(markers[0].starts_with("0;__VYBER_CWD__"));
            assert!(markers[1].starts_with("0;__VYBER_CWD2__"));
        }
        // An absent Windows client leaves the shell's normal SSH resolution alone.
        let output = std::process::Command::new("bash")
            .args([
                "--noprofile",
                "--norc",
                "-c",
                "pwd() { builtin pwd; }; eval \"$1\"; ! declare -F ssh",
                "vyber-test",
                &windows_bash_prompt("", None),
            ])
            .output()?;
        assert!(output.status.success(), "{output:?}");
        Ok(())
    }

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
