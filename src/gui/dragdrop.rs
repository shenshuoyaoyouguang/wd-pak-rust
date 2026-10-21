//! 拖放纯逻辑（G6-05）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropTarget {
    PackageList,
    Workbench,
    OutDir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropAction {
    OpenPak,
    PackFromDir,
    ExportToDir,
    ReplaceEntry,
    Rejected(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropPath {
    pub path: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropPayload {
    pub paths: Vec<DropPath>,
}

pub fn classify_drop(target: DropTarget, payload: &DropPayload) -> DropAction {
    match target {
        DropTarget::Workbench => {
            if payload.paths.iter().any(|p| is_pak(&p.path)) {
                DropAction::OpenPak
            } else if payload.paths.iter().any(|p| p.is_dir) {
                DropAction::PackFromDir
            } else {
                DropAction::Rejected("无法识别".into())
            }
        }
        DropTarget::PackageList => {
            if payload.paths.iter().any(|p| p.is_dir) {
                DropAction::Rejected("目录不能替换包内条目".into())
            } else if payload.paths.len() == 1 {
                DropAction::ReplaceEntry
            } else {
                DropAction::Rejected("一次只能替换一个文件".into())
            }
        }
        DropTarget::OutDir => DropAction::ExportToDir,
    }
}

fn is_pak(path: &str) -> bool {
    path.to_lowercase().ends_with(".pak")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> DropPath {
        DropPath {
            path: path.into(),
            is_dir: false,
        }
    }

    fn dir(path: &str) -> DropPath {
        DropPath {
            path: path.into(),
            is_dir: true,
        }
    }

    #[test]
    fn drop_pak_opens() {
        let p = DropPayload { paths: vec![file("a.pak")] };
        assert_eq!(classify_drop(DropTarget::Workbench, &p), DropAction::OpenPak);
    }

    #[test]
    fn drop_pak_suffix_is_case_insensitive() {
        let p = DropPayload { paths: vec![file("A.PAK")] };
        assert_eq!(classify_drop(DropTarget::Workbench, &p), DropAction::OpenPak);
    }

    #[test]
    fn drop_dir_packs_from_dir() {
        let p = DropPayload { paths: vec![dir(r"C:\some.dir\assets")] };
        assert_eq!(classify_drop(DropTarget::Workbench, &p), DropAction::PackFromDir);
    }

    #[test]
    fn drop_extensionless_file_is_rejected() {
        let p = DropPayload { paths: vec![file(r"C:\some.dir\readme")] };
        assert!(matches!(
            classify_drop(DropTarget::Workbench, &p),
            DropAction::Rejected(_)
        ));
    }

    #[test]
    fn drop_multiple_rejected_for_replace() {
        let p = DropPayload { paths: vec![file("a.txt"), file("b.txt")] };
        assert!(matches!(classify_drop(DropTarget::PackageList, &p), DropAction::Rejected(_)));
    }

    #[test]
    fn drop_directory_rejected_for_replace() {
        let p = DropPayload { paths: vec![dir("a")] };
        assert!(matches!(classify_drop(DropTarget::PackageList, &p), DropAction::Rejected(_)));
    }

    #[test]
    fn drop_single_file_replaces() {
        let p = DropPayload { paths: vec![file("a.txt")] };
        assert_eq!(classify_drop(DropTarget::PackageList, &p), DropAction::ReplaceEntry);
    }
}
