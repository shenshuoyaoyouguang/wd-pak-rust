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
pub struct DropPayload {
    pub paths: Vec<String>,
}

pub fn classify_drop(target: DropTarget, payload: &DropPayload) -> DropAction {
    match target {
        DropTarget::Workbench => {
            if payload.paths.iter().any(|p| p.ends_with(".pak")) {
                DropAction::OpenPak
            } else if payload.paths.iter().any(|p| !p.contains('.')) {
                DropAction::PackFromDir
            } else {
                DropAction::Rejected("无法识别".into())
            }
        }
        DropTarget::PackageList => {
            if payload.paths.len() == 1 {
                DropAction::ReplaceEntry
            } else {
                DropAction::Rejected("一次只能替换一个文件".into())
            }
        }
        DropTarget::OutDir => DropAction::ExportToDir,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_pak_opens() {
        let p = DropPayload { paths: vec!["a.pak".into()] };
        assert_eq!(classify_drop(DropTarget::Workbench, &p), DropAction::OpenPak);
    }

    #[test]
    fn drop_multiple_rejected_for_replace() {
        let p = DropPayload { paths: vec!["a.txt".into(), "b.txt".into()] };
        assert!(matches!(classify_drop(DropTarget::PackageList, &p), DropAction::Rejected(_)));
    }
}
