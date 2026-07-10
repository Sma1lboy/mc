use super::*;

/// 缓存文件路径:`<cache_dir>/<provider>/project/<sanitized-id>.json`。
/// id 来自平台(Modrinth slug/id 或 CurseForge 数字 id),仍过滤一遍只留文件名安全字符。
pub(crate) fn project_cache_path(cache_dir: &std::path::Path, provider: &str, id: &str) -> std::path::PathBuf {
    let safe: String = id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    cache_dir.join(provider).join("project").join(format!("{safe}.json"))
}

/// 带抓取时间戳的缓存包裹体。
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedProject {
    /// 抓取时刻(Unix 秒)。用于 ttl 判断。
    fetched_at: u64,
    data: ProjectDetail,
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 读缓存:`ttl = Some(d)` 时只返回新鲜的(age < d);`ttl = None` 时无视年龄返回(stale 回退)。
/// 文件缺失/损坏/反序列化失败都安静返回 None。
pub(crate) fn read_project_cache(path: &std::path::Path, ttl: Option<std::time::Duration>) -> Option<ProjectDetail> {
    let bytes = std::fs::read(path).ok()?;
    let cached: CachedProject = serde_json::from_slice(&bytes).ok()?;
    if let Some(ttl) = ttl {
        let age = now_unix_secs().saturating_sub(cached.fetched_at);
        if age >= ttl.as_secs() {
            return None;
        }
    }
    Some(cached.data)
}

/// 写缓存(best-effort:建目录 + 写文件,失败仅放弃,不影响主流程)。
pub(crate) fn write_project_cache(path: &std::path::Path, data: &ProjectDetail) {
    let wrapped = CachedProject { fetched_at: now_unix_secs(), data: data.clone() };
    let Ok(json) = serde_json::to_vec(&wrapped) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, json);
}
