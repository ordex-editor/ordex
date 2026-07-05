//! Include-file path resolution and file loading helpers.

use crate::toml_like_parser::{ParsedDocument, parse_reader};
use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};

/// Open a config file and parse it through the reader-based parser entry point.
pub(crate) fn parse_config_file(path: &Path) -> io::Result<ParsedDocument> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    parse_reader(path, reader)
}

/// Resolve an include path relative to the main config file location.
pub(crate) fn resolve_include_path(base_path: &Path, include_path: &str) -> PathBuf {
    let include = PathBuf::from(include_path);
    if include.is_absolute() {
        return include;
    }
    base_path
        .parent()
        .map(|parent| parent.join(&include))
        .unwrap_or(include)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toml_like_parser::ParsedValue;
    use test_utils::TempTree;

    #[test]
    fn resolves_relative_path_from_base_parent() {
        let base = Path::new("/tmp/a/main.cfg");
        assert_eq!(
            resolve_include_path(base, "extra.cfg"),
            PathBuf::from("/tmp/a/extra.cfg")
        );
    }

    #[test]
    fn parses_config_file_without_reading_whole_string_first() {
        let tree = TempTree::new().expect("temp tree");
        tree.write_file(
            "streaming.cfg",
            r#"
[editor]
scroll_margin = 3
"#,
        )
        .expect("write config");
        let path = tree.path().join("streaming.cfg");

        let doc = parse_config_file(&path).expect("parse config file");
        let editor = doc
            .sections
            .iter()
            .find(|section| section.name == "editor")
            .expect("editor section");
        assert_eq!(editor.items.len(), 1);
        assert_eq!(editor.items[0].value, ParsedValue::Integer(3));
    }
}
