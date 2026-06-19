# 模块 · 账号认证

> 三种账号:离线、微软正版、外置登录(Yggdrasil)。微软链路最复杂,做成步骤链。

## 1. 微软正版登录链路(核心)

微软登录是一串串行的 token 交换,做成可插拔的 step 链:

```
① 拿 Microsoft access token
   方式 A: 授权码流(弹浏览器 / 内嵌 WebView,回调 localhost 拿 code → 换 token)
   方式 B: 设备码流(显示 code + 网址,用户手机扫码,轮询拿 token)—— 无浏览器环境友好
        ↓ (得到 MS access_token + refresh_token)
② Xbox Live 认证
   POST user.auth.xboxlive.com/user/authenticate (用 MS token)
        ↓ (得到 Xbox token + uhs 用户哈希)
③ XSTS 授权
   POST xsts.auth.xboxlive.com/xsts/authorize
        ↓ (得到 XSTS token;此处会返回"未成年/无 Xbox 账号"等错误码)
④ 换 Minecraft access token
   POST api.minecraftservices.com/authentication/login_with_xbox
        ↓ (得到 Minecraft accessToken)
⑤ 校验游戏所有权(entitlements)
   GET api.minecraftservices.com/entitlements/mcstore
        ↓ (确认 ownsMinecraft)
⑥ 拿 Minecraft profile
   GET api.minecraftservices.com/minecraft/profile
        ↓ (得到 uuid + name + skins)
```

**刷新**:保存 `refresh_token`,过期时从步骤①方式 A 的 refresh 分支重新走 ①→⑥,**不需要再弹浏览器**。

**OAuth 安全**:用授权码流时配合 **PKCE**(code_verifier/challenge),避免 code 被截获。PCL-CE 的 `IdentityModel/Extensions/Pkce` 是参考。

| 实现 | 对应 |
|------|------|
| Prism | `auth/AuthFlow` + `auth/steps/`(MSAStep / MSADeviceCodeStep / XboxAuthorizationStep / XboxUserStep / MinecraftProfileStep / EntitlementsStep) |
| PCL-CE | `PCL.Core/Minecraft/IdentityModel/OAuth/Client`、`Extensions/OpenId`、`Extensions/Pkce` |
| PCL2 | `ModLaunch.vb` 的 `McLoginMs`(OAuth2 device flow) |

## 2. 离线登录

- 用户输入用户名 → UUID = 用户名的 MD5(offline 命名空间惯例)。
- 无网络验证,`accessToken` 给个占位值,`userType=legacy`。
- 注意:正版服务器会拒绝离线账号;仅用于单机/盗版/离线服。

## 3. 外置登录(Yggdrasil / Authlib-Injector)🌟

国内私服、第三方皮肤站(LittleSkin 等)用的认证协议。

```
POST <authserver>/authenticate
  { username, password, clientToken, agent: { name: "Minecraft", version: 1 } }
→ { accessToken, clientToken, selectedProfile: { id, name }, availableProfiles }
```

启动时需要额外注入 **authlib-injector** 的 javaagent:
```
-javaagent:authlib-injector.jar=<authserver-url>
```
这让游戏把正版验证请求重定向到第三方服务器。

| 实现 | 对应 |
|------|------|
| PCL-CE | `IdentityModel/Yggdrasil/Client`、`Extensions/YggdrasilConnect` |
| PCL2 | `PageLoginAuth`(外置)、`PageLoginNide`(统一通行证) |

## 4. 账号存储

- 多账号列表持久化(JSON 或加密存储)。
- **敏感字段(accessToken / refreshToken)必须加密**。PCL2 用 DES(偏弱,自研建议用系统 keychain / DPAPI / AES)。
- 存:type、内部 id、profileId(uuid)、profileName、token、refreshToken、所有权信息。

```jsonc
{
  "accounts": [
    { "type": "msa", "internalId": "...", "profileId": "<uuid>",
      "profileName": "Steve", "accessToken": "<enc>", "refreshToken": "<enc>",
      "ownsMinecraft": true }
  ]
}
```

| 实现 | 对应 |
|------|------|
| Prism | `auth/AccountData`、`auth/AccountList`、`auth/MinecraftAccount`,存 `accounts.json` |
| PCL2 | 注册表加密存储 `CacheMsV2*` |

## 5. 自研要点

1. **认证做成 step 链**,和启动 step 链同一套框架。每步输入输出明确,失败能定位到具体步骤。
2. **device code flow 一定要做**——比内嵌 WebView 省心,且无头/跨平台环境可用。
3. **XSTS 错误码要翻译成人话**(如 `2148916233` = 没有 Xbox 账号、`2148916238` = 未成年需家长同意)。
4. **token 加密用平台原生方案**(Windows DPAPI / macOS Keychain / Linux Secret Service),别自己滚 DES。
5. **离线和外置共用同一个 AuthSession 出口**(token/uuid/name),让启动阶段无感知账号类型。
