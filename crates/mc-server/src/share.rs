//! Instance/modpack sharing, persisted in the `shares` table (sqlx). A user
//! publishes an instance's metadata + file manifest and gets a short content-
//! derived id others fetch and rebuild from.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// One declared file in a shared instance (downloaded by url, verified by sha1).
#[derive(Serialize, Deserialize, Clone)]
pub struct SharedFile {
    pub path: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
}

/// A shared instance: enough to recreate it elsewhere.
#[derive(Serialize, Deserialize, Clone)]
pub struct SharedInstance {
    pub name: String,
    pub mc_version: String,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub files: Vec<SharedFile>,
    /// Server-assigned; ignored on submit.
    #[serde(default)]
    pub id: String,
}

/// DB-backed share registry. Ids are derived deterministically from content so
/// resubmitting the same instance returns the same id (idempotent).
#[derive(Clone)]
pub struct ShareStore {
    pool: PgPool,
}

impl ShareStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn put(&self, mut inst: SharedInstance) -> anyhow::Result<String> {
        let id = derive_id(&inst);
        inst.id = id.clone();
        let json = serde_json::to_string(&inst)?;
        sqlx::query("INSERT INTO shares (id, json) VALUES ($1, $2) ON CONFLICT (id) DO UPDATE SET json = EXCLUDED.json")
            .bind(&id)
            .bind(&json)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    pub async fn get(&self, id: &str) -> Option<SharedInstance> {
        let row: Option<(String,)> = sqlx::query_as("SELECT json FROM shares WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();
        row.and_then(|(json,)| serde_json::from_str(&json).ok())
    }

    /// Store an opaque JSON blob (e.g. an agent chat transcript) under a
    /// content-derived id, reusing the same `shares` table. Idempotent: the same
    /// payload yields the same id. Used for public conversation sharing.
    pub async fn put_raw(&self, value: &serde_json::Value) -> anyhow::Result<String> {
        let json = serde_json::to_string(value)?;
        let id = derive_raw_id(&json);
        sqlx::query("INSERT INTO shares (id, json) VALUES ($1, $2) ON CONFLICT (id) DO UPDATE SET json = EXCLUDED.json")
            .bind(&id)
            .bind(&json)
            .execute(&self.pool)
            .await?;
        Ok(id)
    }

    /// Fetch a raw JSON blob previously stored with [`put_raw`].
    pub async fn get_raw(&self, id: &str) -> Option<serde_json::Value> {
        let row: Option<(String,)> = sqlx::query_as("SELECT json FROM shares WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();
        row.and_then(|(json,)| serde_json::from_str(&json).ok())
    }
}

/// Render a shared conversation (`{ messages: UIMessage[], title? }`) as a
/// self-contained public HTML page that mirrors the desktop chat UI (方块工坊
/// skin): user bubbles right, assistant panels left, markdown via `marked`,
/// tool calls + ask_user options shown read-only. The payload is embedded and
/// rendered client-side (keeps this a single static template).
pub fn render_conversation_html(value: &serde_json::Value) -> String {
    // Embed the payload for client-side render; neutralise `</` so a `</script>`
    // inside any string can't break out of the script element.
    let data = serde_json::to_string(value).unwrap_or_else(|_| "null".into()).replace("</", "<\\/");
    format!(
        r##"<!doctype html>
<html lang="zh"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>kobeMC · 整合包助手对话</title>
<script src="https://cdn.jsdelivr.net/npm/marked/marked.min.js"></script>
<style>
  :root {{
    --bg:#16170f; --border:#0c0d0a; --panel:#1d1f15; --panel-2:#2a2c20;
    --accent:#e8590c; --accent-text:#16170f;
    --text:#f1ead8; --strong:#f3ecda; --sub:#a39f8e; --muted:#8f8f7e;
  }}
  * {{ box-sizing:border-box; }}
  body {{ margin:0; background:var(--bg); color:var(--text);
    font:14px/1.7 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,"PingFang SC","Microsoft YaHei",sans-serif; }}
  header {{ border-bottom:2px solid var(--border); padding:16px 28px; }}
  header h1 {{ margin:0; font-size:17px; color:var(--strong); }}
  header .sub {{ margin-top:4px; font-size:12px; color:var(--muted); }}
  main {{ max-width:820px; margin:0 auto; padding:24px 28px; display:flex; flex-direction:column; gap:18px; }}
  .row {{ display:flex; }}
  .row.user {{ justify-content:flex-end; }}
  .row.assistant {{ justify-content:flex-start; }}
  .bubble-user {{ max-width:min(80%,600px); padding:9px 13px; background:var(--accent);
    color:var(--accent-text); font-size:14px; white-space:pre-wrap; word-break:break-word; }}
  .panel {{ max-width:min(85%,760px); min-width:0; padding:11px 14px; background:var(--panel);
    box-shadow:inset 0 0 0 1px rgba(0,0,0,.35); }}
  .md {{ font-size:14px; color:var(--text); word-break:break-word; }}
  .md.reasoning {{ color:var(--muted); font-size:12px; }}
  .md :is(h1,h2,h3) {{ font-size:15px; color:var(--strong); margin:.6em 0 .3em; }}
  .md p {{ margin:.4em 0; }}
  .md code {{ background:var(--panel-2); padding:1px 5px; font-size:12.5px; }}
  .md pre {{ background:var(--panel-2); padding:10px; overflow-x:auto; }}
  .md pre code {{ background:none; padding:0; }}
  .md table {{ border-collapse:collapse; width:100%; font-size:.9em; margin:.7em 0; }}
  .md th,.md td {{ border-bottom:1px solid var(--border); padding:6px 10px; text-align:left; }}
  .md th {{ background:var(--panel-2); color:var(--strong); }}
  .tool {{ display:inline-block; margin:3px 0; padding:4px 10px; background:var(--panel-2);
    color:var(--sub); font-size:12px; }}
  .ask-q {{ font-size:13px; color:var(--text); margin:6px 0; }}
  .opt {{ margin:6px 0; padding:8px 11px; background:var(--panel-2);
    box-shadow:inset 0 0 0 1px var(--border); }}
  .opt-label {{ font-size:13px; font-weight:600; color:var(--strong); }}
  .opt-desc {{ margin-top:2px; font-size:12px; color:var(--muted); }}
  footer {{ max-width:820px; margin:0 auto; padding:8px 28px 32px; color:var(--muted); font-size:11px; }}
</style>
</head><body>
<header><h1 id="title">整合包助手对话</h1><div class="sub">kobeMC · 只读分享</div></header>
<main id="conv"></main>
<footer>由 kobeMC 分享 · 只读视图</footer>
<script>
  const conv = {data};
  const root = document.getElementById('conv');
  function el(tag, cls, child) {{ const e=document.createElement(tag); if(cls)e.className=cls;
    if(child!=null) e.appendChild(typeof child==='string'?document.createTextNode(child):child); return e; }}
  function md(text) {{ const d=el('div','md'); d.innerHTML=window.marked?marked.parse(text||''):(text||''); return d; }}
  if (conv && conv.title) document.getElementById('title').textContent = conv.title;
  (conv && conv.messages || []).forEach(function(m) {{
    if (m.role === 'user') {{
      const txt=(m.parts||[]).filter(p=>p.type==='text').map(p=>p.text).join('');
      root.appendChild(el('div','row user', el('div','bubble-user', txt))); return;
    }}
    const panel=el('div','panel');
    (m.parts||[]).forEach(function(p) {{
      if (p.type==='text') panel.appendChild(md(p.text));
      else if (p.type==='reasoning') {{ const d=md(p.text); d.classList.add('reasoning'); panel.appendChild(d); }}
      else if (p.type==='tool-ask_user_question') {{
        const inp=p.input||{{}};
        if (inp.question) panel.appendChild(el('div','ask-q', inp.question));
        (inp.options||[]).forEach(function(o) {{
          const opt=el('div','opt'); opt.appendChild(el('div','opt-label', o.label||''));
          if (o.description) opt.appendChild(el('div','opt-desc', o.description)); panel.appendChild(opt);
        }});
      }}
      else if (typeof p.type==='string' && p.type.indexOf('tool-')===0)
        panel.appendChild(el('div','tool', '🔧 '+p.type.slice(5)));
    }});
    root.appendChild(el('div','row assistant', panel));
  }});
</script>
</body></html>"##
    )
}

/// Content-derived id for a raw blob (same FNV-1a scheme as `derive_id`, over
/// the serialized JSON). Prefixed `c` so conversation ids don't collide with
/// instance ids in the shared table.
fn derive_raw_id(json: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in json.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("c{h:016x}")
}

/// Short stable id from the instance's defining fields (name+version+files).
/// A tiny FNV-1a hash keeps it dependency-free.
fn derive_id(inst: &SharedInstance) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut feed = |s: &str| {
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    feed(&inst.name);
    feed(&inst.mc_version);
    feed(inst.loader.as_deref().unwrap_or(""));
    for f in &inst.files {
        feed(&f.path);
        feed(&f.url);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SharedInstance {
        SharedInstance {
            name: "Pack".into(),
            mc_version: "1.20.1".into(),
            loader: Some("fabric".into()),
            loader_version: None,
            files: vec![],
            id: String::new(),
        }
    }

    #[tokio::test]
    async fn put_get_roundtrip_and_stable_id() {
        let Some(pool) = crate::db::test_pool().await else { return };
        let store = ShareStore::new(pool);
        let id1 = store.put(sample()).await.unwrap();
        let id2 = store.put(sample()).await.unwrap();
        assert_eq!(id1, id2); // deterministic / idempotent
        assert_eq!(store.get(&id1).await.unwrap().name, "Pack");
        assert!(store.get("nonexistent").await.is_none());
    }
}
