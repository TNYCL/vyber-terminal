//! Embedded icons and the file-type → icon mapping used by the file browser.
//!
//! File icons are Seti UI glyphs (MIT); interface icons are Lucide (ISC), with
//! `panel-dock` and `panel-float` drawn in the same style. All are monochrome
//! and tinted at draw time. Regenerate with
//! `scripts/build_file_icons.py`.
use gpui::{AssetSource, SharedString};
use std::{borrow::Cow, path::Path};

macro_rules! embed {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/", $name)))),*]
    };
}

static ICONS: &[(&str, &[u8])] = embed![
    "files/audio.svg",
    "files/c-sharp.svg",
    "files/c.svg",
    "files/config.svg",
    "files/cpp.svg",
    "files/css.svg",
    "files/csv.svg",
    "files/dart.svg",
    "files/db.svg",
    "files/docker.svg",
    "files/elixir.svg",
    "files/eslint.svg",
    "files/favicon.svg",
    "files/font.svg",
    "files/git_ignore.svg",
    "files/go.svg",
    "files/graphql.svg",
    "files/haskell.svg",
    "files/html.svg",
    "files/image.svg",
    "files/info.svg",
    "files/java.svg",
    "files/javascript.svg",
    "files/json.svg",
    "files/kotlin.svg",
    "files/less.svg",
    "files/license.svg",
    "files/lock.svg",
    "files/lua.svg",
    "files/makefile.svg",
    "files/markdown.svg",
    "files/notebook.svg",
    "files/npm.svg",
    "files/pdf.svg",
    "files/php.svg",
    "files/powershell.svg",
    "files/prisma.svg",
    "files/python.svg",
    "files/react.svg",
    "files/ruby.svg",
    "files/rust.svg",
    "files/sass.svg",
    "files/scala.svg",
    "files/shell.svg",
    "files/svelte.svg",
    "files/svg.svg",
    "files/swift.svg",
    "files/terraform.svg",
    "files/tex.svg",
    "files/tsconfig.svg",
    "files/typescript.svg",
    "files/video.svg",
    "files/vite.svg",
    "files/vue.svg",
    "files/wasm.svg",
    "files/windows.svg",
    "files/xml.svg",
    "files/yarn.svg",
    "files/yml.svg",
    "files/zig.svg",
    "files/zip.svg",
    "ui/app-window.svg",
    "ui/archive.svg",
    "ui/arrow-down.svg",
    "ui/arrow-left.svg",
    "ui/arrow-up.svg",
    "ui/book-open.svg",
    "ui/check.svg",
    "ui/chevron-down.svg",
    "ui/chevron-right.svg",
    "ui/chevron-up.svg",
    "ui/chevrons-down-up.svg",
    "ui/chevrons-up-down.svg",
    "ui/circle-alert.svg",
    "ui/circle-check.svg",
    "ui/circle-dot.svg",
    "ui/cloud-download.svg",
    "ui/cloud-upload.svg",
    "ui/cloud.svg",
    "ui/code.svg",
    "ui/columns-2.svg",
    "ui/copy.svg",
    "ui/ellipsis.svg",
    "ui/external-link.svg",
    "ui/eye.svg",
    "ui/file-diff.svg",
    "ui/file-text.svg",
    "ui/file-x.svg",
    "ui/file.svg",
    "ui/files.svg",
    "ui/flag.svg",
    "ui/fold-vertical.svg",
    "ui/folder-git-2.svg",
    "ui/folder-git.svg",
    "ui/folder-open.svg",
    "ui/folder-plus.svg",
    "ui/folder-search.svg",
    "ui/folder.svg",
    "ui/git-branch-plus.svg",
    "ui/git-branch.svg",
    "ui/git-commit-horizontal.svg",
    "ui/git-compare.svg",
    "ui/git-fork.svg",
    "ui/git-graph.svg",
    "ui/git-merge.svg",
    "ui/image.svg",
    "ui/loader-circle.svg",
    "ui/menu.svg",
    "ui/maximize-2.svg",
    "ui/minimize-2.svg",
    "ui/minus.svg",
    "ui/panel-dock.svg",
    "ui/panel-float.svg",
    "ui/panel-right-close.svg",
    "ui/panel-right.svg",
    "ui/pencil.svg",
    "ui/pin-off.svg",
    "ui/pin.svg",
    "ui/plus.svg",
    "ui/refresh-ccw.svg",
    "ui/refresh-cw.svg",
    "ui/rotate-ccw.svg",
    "ui/rows-2.svg",
    "ui/save.svg",
    "ui/scan.svg",
    "ui/search.svg",
    "ui/square-terminal.svg",
    "ui/tag.svg",
    "ui/text-search.svg",
    "ui/trash.svg",
    "ui/triangle-alert.svg",
    "ui/undo-2.svg",
    "ui/unfold-vertical.svg",
    "ui/wrap-text.svg",
    "ui/x.svg",
    "ui/zoom-in.svg",
    "ui/zoom-out.svg",
];

/// Serves Vyber's icons under `vyber/…` and defers everything else to GPUI Kit.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        if let Some(name) = path.strip_prefix("vyber/") {
            return Ok(ICONS
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, bytes)| Cow::Borrowed(*bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        let mut list = gpui_kit::assets::Assets.list(path)?;
        list.extend(
            ICONS
                .iter()
                .map(|(n, _)| format!("vyber/{n}"))
                .filter(|n| n.starts_with(path))
                .map(SharedString::from),
        );
        Ok(list)
    }
}

const BLUE: u32 = 0x56a8d8;
const GREEN: u32 = 0x8dc149;
const YELLOW: u32 = 0xd8cb5e;
const ORANGE: u32 = 0xe37933;
const RED: u32 = 0xe0525a;
const PINK: u32 = 0xf55385;
const PURPLE: u32 = 0xa888d8;
const GREY: u32 = 0x8a959a;
const RUST: u32 = 0xffa359;
const GIT: u32 = 0xe8643c;
const MARKDOWN: u32 = 0x6cc07a;

/// Icon path and tint for a file. Folders are drawn with chevrons only.
pub fn file_icon(path: &Path) -> (SharedString, u32) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let (icon, color) = by_name(&name).unwrap_or_else(|| {
        let extension = name.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
        by_extension(extension)
    });
    let path = if icon.contains('/') {
        format!("vyber/{icon}.svg")
    } else {
        format!("vyber/files/{icon}.svg")
    };
    (path.into(), color)
}

fn by_name(name: &str) -> Option<(&'static str, u32)> {
    Some(match name {
        "dockerfile" | "containerfile" => ("docker", BLUE),
        ".dockerignore" => ("docker", GREY),
        n if n.starts_with("dockerfile.") || n.ends_with(".dockerfile") => ("docker", BLUE),
        n if (n.starts_with("compose.") || n.starts_with("docker-compose"))
            && (n.ends_with(".yml") || n.ends_with(".yaml")) =>
        {
            ("docker", BLUE)
        }
        ".gitignore" | ".gitattributes" | ".gitmodules" | ".gitkeep" | ".git-blame-ignore-revs" => {
            ("git_ignore", GIT)
        }
        "makefile" | "gnumakefile" | "cmakelists.txt" => ("makefile", ORANGE),
        "license" | "licence" | "copying" | "license.md" | "license.txt" | "licence.md"
        | "unlicense" => ("license", YELLOW),
        "readme" | "readme.txt" => ("info", BLUE),
        "package.json" | ".npmrc" | ".npmignore" => ("npm", RED),
        "yarn.lock" | ".yarnrc" | ".yarnrc.yml" => ("yarn", BLUE),
        "cargo.toml" => ("rust", RUST),
        "go.mod" | "go.sum" | "go.work" => ("go", BLUE),
        ".editorconfig" => ("config", GREY),
        "favicon.ico" => ("favicon", YELLOW),
        n if n.starts_with("tsconfig") && n.ends_with(".json") => ("tsconfig", BLUE),
        n if n.starts_with("vite.config.") => ("vite", YELLOW),
        n if n.starts_with(".eslintrc") || n.starts_with("eslint.config.") => ("eslint", PURPLE),
        n if n.starts_with(".env") => ("config", GREY),
        _ => return None,
    })
}

fn by_extension(extension: &str) -> (&'static str, u32) {
    match extension {
        "md" | "markdown" | "mdx" => ("markdown", MARKDOWN),
        "rs" => ("rust", RUST),
        "go" => ("go", BLUE),
        "ts" | "mts" | "cts" => ("typescript", BLUE),
        "tsx" | "jsx" => ("react", BLUE),
        "js" | "mjs" | "cjs" => ("javascript", YELLOW),
        "py" | "pyi" | "pyw" => ("python", BLUE),
        "ipynb" => ("notebook", BLUE),
        "json" | "jsonc" | "json5" => ("json", YELLOW),
        "toml" | "ini" | "cfg" | "conf" | "properties" => ("config", GREY),
        "yml" | "yaml" => ("yml", PURPLE),
        "lock" => ("lock", GREY),
        "sh" | "bash" | "zsh" | "fish" => ("shell", GREEN),
        "ps1" | "psm1" | "psd1" => ("powershell", BLUE),
        "bat" | "cmd" | "exe" | "msi" | "dll" => ("windows", BLUE),
        "html" | "htm" => ("html", ORANGE),
        "css" => ("css", BLUE),
        "scss" | "sass" => ("sass", PINK),
        "less" => ("less", BLUE),
        "svg" => ("svg", PURPLE),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "icns" | "tiff" | "avif" => {
            ("image", PURPLE)
        }
        "sql" | "db" | "sqlite" | "sqlite3" => ("db", PINK),
        "xml" | "plist" | "xaml" | "csproj" => ("xml", ORANGE),
        "csv" | "tsv" => ("csv", GREEN),
        "zip" | "tar" | "gz" | "tgz" | "7z" | "rar" | "xz" | "bz2" => ("zip", GREY),
        "pdf" => ("pdf", RED),
        "java" | "jar" => ("java", RED),
        "kt" | "kts" => ("kotlin", ORANGE),
        "c" | "h" => ("c", BLUE),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => ("cpp", BLUE),
        "cs" => ("c-sharp", BLUE),
        "rb" => ("ruby", RED),
        "php" => ("php", PURPLE),
        "swift" => ("swift", ORANGE),
        "lua" => ("lua", BLUE),
        "zig" => ("zig", ORANGE),
        "vue" => ("vue", GREEN),
        "svelte" => ("svelte", RED),
        "dart" => ("dart", BLUE),
        "ex" | "exs" => ("elixir", PURPLE),
        "hs" => ("haskell", PURPLE),
        "scala" | "sc" => ("scala", RED),
        "tf" | "tfvars" => ("terraform", PURPLE),
        "graphql" | "gql" => ("graphql", PINK),
        "wasm" | "wat" => ("wasm", PURPLE),
        "ttf" | "otf" | "woff" | "woff2" | "eot" => ("font", RED),
        "mp3" | "wav" | "ogg" | "flac" | "m4a" => ("audio", PURPLE),
        "mp4" | "mov" | "webm" | "mkv" | "avi" => ("video", PINK),
        "prisma" => ("prisma", GREY),
        "tex" | "bib" => ("tex", GREEN),
        "txt" | "log" | "rst" => ("ui/file-text", GREY),
        _ => ("ui/file", GREY),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_win_over_extensions() {
        assert_eq!(
            file_icon(Path::new("a/Dockerfile")).0,
            "vyber/files/docker.svg"
        );
        assert_eq!(
            file_icon(Path::new("compose.local-auth.yaml")).0,
            "vyber/files/docker.svg"
        );
        assert_eq!(
            file_icon(Path::new(".gitignore")).0,
            "vyber/files/git_ignore.svg"
        );
        assert_eq!(
            file_icon(Path::new("README.md")).0,
            "vyber/files/markdown.svg"
        );
        assert_eq!(
            file_icon(Path::new("src/main.rs")).0,
            "vyber/files/rust.svg"
        );
        assert_eq!(file_icon(Path::new("notes")).0, "vyber/ui/file.svg");
        assert_eq!(file_icon(Path::new("ci.yml")).0, "vyber/files/yml.svg");
    }

    #[test]
    fn every_mapped_icon_is_embedded() {
        let names = [
            "a.md",
            "a.rs",
            "a.go",
            "a.ts",
            "a.tsx",
            "a.js",
            "a.py",
            "a.ipynb",
            "a.json",
            "a.toml",
            "a.yml",
            "a.lock",
            "a.sh",
            "a.ps1",
            "a.bat",
            "a.html",
            "a.css",
            "a.scss",
            "a.less",
            "a.svg",
            "a.png",
            "a.sql",
            "a.xml",
            "a.csv",
            "a.zip",
            "a.pdf",
            "a.java",
            "a.kt",
            "a.c",
            "a.cpp",
            "a.cs",
            "a.rb",
            "a.php",
            "a.swift",
            "a.lua",
            "a.zig",
            "a.vue",
            "a.svelte",
            "a.dart",
            "a.ex",
            "a.hs",
            "a.scala",
            "a.tf",
            "a.graphql",
            "a.wasm",
            "a.ttf",
            "a.mp3",
            "a.mp4",
            "a.prisma",
            "a.tex",
            "a.txt",
            "a.unknown",
            "Dockerfile",
            ".gitignore",
            "Makefile",
            "LICENSE",
            "README",
            "package.json",
            "yarn.lock",
            "Cargo.toml",
            "go.mod",
            ".editorconfig",
            "favicon.ico",
            "tsconfig.json",
            "vite.config.ts",
            ".eslintrc.json",
            ".env",
        ];
        for name in names {
            let (path, _) = file_icon(Path::new(name));
            assert!(
                Assets.load(&path).unwrap().is_some(),
                "{name} maps to missing {path}"
            );
        }
    }
}
