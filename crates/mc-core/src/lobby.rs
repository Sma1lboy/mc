//! 联机大厅(EasyTier room)凭据。一个领域 = 一个虚拟局域网房间:所有成员从 mc-server
//! 拿到**相同**的 `network_name` + `network_secret`(成员鉴权后才发),进同一个 EasyTier
//! 网络;`nodes` 是会合 / 中继的 external-node 列表(P2P 公共节点 + 可选的我们自建中继)。
//!
//! 这里只负责**取凭据**(P1)。真正拉起 EasyTier(建 TUN、加入网络)走后续阶段的特权 helper。

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::server::ServerClient;

/// 一个 EasyTier external/relay 节点。`kind`:`"p2p"` = 公共共享节点(打洞后直连,我们零成本);
/// `"hosted"` = 我们自建的中继(host 点,打洞失败时兜底)。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LobbyNode {
    pub kind: String,
    pub name: String,
    pub addr: String,
}

/// 某领域的联机大厅凭据(EasyTier 房间)。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct LobbyCreds {
    pub network_name: String,
    pub network_secret: String,
    #[serde(default)]
    pub nodes: Vec<LobbyNode>,
}

impl ServerClient {
    /// 取某领域的联机大厅凭据(成员鉴权;非成员 403)。
    pub async fn realm_lobby(&self, realm_id: &str) -> Result<LobbyCreds> {
        self.get_json(&format!("/v1/realms/{realm_id}/lobby")).await
    }
}
