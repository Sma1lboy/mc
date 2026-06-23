//! 整合包更新检查:对比实例的安装来源版本与平台上的可用版本,挑出更新的那些。
//!
//! 仅做「检查」这一只读、纯逻辑的部分(实际「应用更新」改实例文件是另一码事,后续单独做)。

use crate::modplatform::modrinth::VersionDetail;

/// 从平台版本列表里挑出比当前安装版本「更新」的那些。
///
/// Modrinth 的版本列表按发布时间倒序(最新在前)。当前实例来源的 `current_version_id`
/// 在列表里定位后,取它**之前**(更新)的所有版本即为可更新项。
///
/// 定位不到(版本被下架,或来源 `version_id` 未知/为 `None`)时返回空 —— 宁可不提示,
/// 也不把整张列表误当成「都比你新」。
pub fn newer_versions(
    versions: Vec<VersionDetail>,
    current_version_id: Option<&str>,
) -> Vec<VersionDetail> {
    let Some(cur) = current_version_id else {
        return Vec::new();
    };
    match versions.iter().position(|v| v.id == cur) {
        Some(pos) => versions.into_iter().take(pos).collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(id: &str) -> VersionDetail {
        VersionDetail {
            id: id.to_string(),
            version_number: id.to_string(),
            name: id.to_string(),
            version_type: "release".to_string(),
            game_versions: vec![],
            loaders: vec![],
            date_published: String::new(),
            downloads: 0,
            changelog: String::new(),
            mrpack_url: None,
            mrpack_filename: None,
            file_size: None,
        }
    }

    #[test]
    fn takes_versions_before_current() {
        // newest-first: c, b, a；当前是 b → 只有 c 更新。
        let r = newer_versions(vec![v("c"), v("b"), v("a")], Some("b"));
        assert_eq!(r.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["c"]);
    }

    #[test]
    fn current_is_newest_means_none() {
        assert!(newer_versions(vec![v("c"), v("b"), v("a")], Some("c")).is_empty());
    }

    #[test]
    fn unknown_or_missing_current_returns_empty() {
        assert!(newer_versions(vec![v("c"), v("b")], Some("zzz")).is_empty());
        assert!(newer_versions(vec![v("c")], None).is_empty());
    }
}
