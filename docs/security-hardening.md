# 安全修复与剩余边界（2026-10-24）

本记录不是“全部漏洞修复”或“可直接上线”的结论。范围为 hbbs、hbbr 和配套客户端；实际验证平台为 Windows x86_64 MSVC。未更改原生打洞、加密协议字段或共享子模块固定提交。

## 本轮已实现

### 认证与传输

- hbbs / hbbr 默认拒绝空白 KEY。只有显式设置 `ALLOW_ANONYMOUS=Y` 才允许空 KEY；迁移时必须给 hbbr 配置与 hbbs 相同的非空 KEY（或共享密钥文件并使用 `-k _`）。不再把旧的空 KEY 默认行为静默保留下来。随附 Compose、systemd hbbr 启动参数及 s6 镜像默认值已同步使用共享密钥；没有在本机执行 Docker / Linux 服务部署。
- 原生 UDP 注册/探测并非所有消息都携带 KEY 凭据；空 KEY 启动拒绝不等于这些入口已经实现全量身份认证，也不等于注册设备自动获得远控权限。若要求封闭设备准入，需要独立的登记/身份方案和协议联调，不能仅增加一个启动参数后声称完成。
- **KEY / 公钥匹配不是人员身份认证**，公钥也不应作为秘密口令。远程会话继续使用客户端原有的密码、人工确认、可选 2FA、白名单和权限校验；本轮没有声称实现企业账号、RBAC、人员离职撤权或客户端设备证书认证。
- WebSocket 服务入口复用客户端已经采用的 tokio-tungstenite / tungstenite 0.29 修补分支，分别固定为 `c241b11b03de4881faf1ecd2fe6e867c92f3013b` / `efce47e5364f00a63302cf07ab1fb13ded478914`；握手超时 10 秒。共享库内部仍有 0.26.2 依赖，没有编辑共享子模块。默认不信任客户端转发地址，只对 `TRUSTED_PROXIES` 指定代理解析来源；转发链从右向左停止于首个不可信跳点。
- 服务端 TCP 控制入口及 hbbs / hbbr WebSocket 准入消息限制为 128 KiB，超大 TCP 声明长度在读取正文前拒绝，WebSocket 同时限制单帧与完整消息，不能用分片绕过。复用客户端传输分支的动态配置能力，中继 TCP / WebSocket 完成准入后恢复媒体报文上限，保留原生/混合中继业务行为；其读取缓冲按实际收到的数据增长，不按远端声明长度一次性扩张。仍未宣称限制总并发连接数或消除全部 DoS。
- 审计传输强制远端 HTTPS、校验证书、禁止重定向、配置代理失败不得绕过代理。仅字面量 loopback IP 允许 HTTP。未改变其他历史 HTTP 接口的兼容性回退行为；这些接口的无效证书兼容模式仍是后续风险项。

### 可持久化审计

- hbbs 可通过 `AUDIT_LISTEN` / `AUDIT_TOKEN` 启用 `/api/audit/conn`、`/api/audit/file`、`/api/audit/alarm` 写入端点。默认关闭；建议只绑定 loopback 并在 HTTPS 反向代理后使用。
- Bearer token 至少 32 个非空白 ASCII 字节，恒定时间比较；未授权请求不写数据库；请求体限 1 MiB，要求非空设备 id 和 UUID nonce。凭据不写入服务日志。
- 复用现有 SQLite 数据库，新增 `audit_event` 表；提交后才返回空 200。`(device_id, nonce)` 唯一：相同内容重试只确认一次，不同内容返回 409，不覆盖原记录。TCP 来源与接收 UTC 时间由服务端填写；设备身份和事件字段仍是客户端声明，不能视为不可抵赖的人员身份。
- 客户端审计先写配置目录旁的 `.audit-outbox`，同步文件并原子改名；失败保留，重启扫描并补传，只有空 2xx 确认才删除。记录包含 nonce 与事件时间，不在 outbox 中保存 Bearer token。
- 补传只向当前配置的审计 URL 发送；历史 URL 或损坏记录保留供检查，不向旧服务泄露新 token。`occurred_at_ms` 是客户端时间，服务端 `received_at` 是接收时间。
- 文件日志按每批 10 项拆分并加批次信息，不再仅记录前 10 个名称。连接、文件、告警生产者沿用原有业务链路。

### 运维日志与依赖

- KEY 日志只输出“已配置”，不输出原值。沿用 flexi_logger，`LOG_DIR` 默认 `logs`；每日或 10 MiB 轮转，保留 31 个历史文件，同时输出 info 及以上至 stdout。31 个文件不等于保证 31 天留存。
- 日志初始化改到配置加载之后，`.env` / 配置文件中的日志路径与等级可生效。
- SQLx 0.6 升至 0.8.6、SQLite 绑定升至 libsqlite3-sys 0.30.1；适配连接及建表 API，保留原 peer 表和参数化 CRUD。Axum 升至 0.8；删除源码未使用的旧 JWT / bcrypt / headers / tower-http / http 直接依赖。锁定 ed25519 1.5.3，避免旧 1.5.0 错选不兼容 signature 主版本。
- 服务端旧 Reqwest fork 仅被版本检查 POST 和测试使用；已改用 Reqwest 0.12，保留平台 native TLS / rustls-native-roots 及系统代理支持，并移除旧 Hyper/h2/rustls 0.21 依赖链，不保留未使用的 blocking/socks 能力。
- 上一轮兼容修补的 openssl、rustls 0.23、crossbeam、tempfile 等更新继续保留。除移除上述不再需要的 Reqwest Git fork、复用上述两个客户端已有且固定提交的传输修补依赖外，其余 Git 依赖修订号和两个共享子模块固定提交未变；开发工具、密钥、数据库不提交到仓库。

## 部署配置

详见 `environment-variables.md` 的 Logging、Keys and encryption 和 Durable audit receiver 章节。客户端需要配置实际 HTTPS `api-server` 和同值 `audit-token`；目前复用现有配置选项机制，没有新增配置 UI。

审计是显式启用项：不设置监听地址和客户端审计服务地址，不会自动生成集中审计记录。非 loopback 监听应只允许 HTTPS 网关访问，禁止明文向公网公开 token。客户端 token 沿用本地配置存储，部署方必须限制配置/日志/SQLite/outbox 的文件 ACL；Windows 权限继承也需要核验。

## 验证证据

测试与检查命令（`--config` 仅为本机临时下载源，不改变源码依赖）：

```powershell
# 服务端全部目标；排除旧的固定文件名 10,000 次插入压测
cargo test --locked --all-targets -- --skip database::tests::test_insert
# 客户端基础模块和直接编译实际 HTTP 工厂的集成测试
cargo test --locked -p base --all-targets -- --include-ignored
```

最终结果：服务端库 20、hbbr 17、集成 3，合计 40 次测试执行通过（部分共享测试在两个目标重复执行）；客户端基础库 24、实际 HTTP 工厂集成 2，合计 26 次通过，无忽略项。完整客户端检查的原生依赖限制见本节末尾。

| 需求 | 精确测试证据 |
| --- | --- |
| TCP 超大长度声明提前拒绝 | `common::tests::control_messages_reject_oversized_headers_before_reading_payloads` |
| hbbs WebSocket 单帧及分片超限拒绝 | `websocket::tests::oversized_websocket_control_messages_are_rejected` |
| 正确凭据完成 TCP / WebSocket / 两种混合配对，并转发大于准入上限的媒体数据 | `relay_server::tests::relay_media_payloads_are_not_limited_to_admission_size` |
| 错误中继凭据在 TCP / WebSocket 均不能进入配对池 | `relay_server::tests::invalid_relay_credentials_never_enter_the_pairing_pool` |
| 默认拒绝空 KEY，显式开放与错误配置分支 | `common::tests::service_admission_requires_key_or_explicit_anonymous_mode`、`relay_startup_rejects_an_empty_key_by_default` |
| 启动日志落盘且不包含配置 KEY | `relay_startup_redacts_the_key_and_writes_a_log_file` |
| 审计配置边界 | `audit::tests::audit_listener_rejects_unusable_credentials_and_addresses` |
| 实际 HTTP 鉴权、非法字段、413、nonce 重试与冲突保护 | `audit::tests::audit_http_requires_credentials_and_persists_idempotently` |
| 原 peer CRUD 及审计去重重启后保持 | `database::tests::peer_updates_and_audit_records_survive_database_reopen` |
| 不可信转发地址与完整代理链 | `websocket::tests::untrusted_peers_cannot_spoof_forwarded_addresses`、`forwarded_chain_stops_at_the_first_untrusted_hop` |
| 实际二进制往返、握手超时 | `websocket::tests::websocket_handshake_preserves_binary_messages`、`stalled_websocket_handshakes_time_out` |
| 审计 URL 禁止远程明文及 URL 凭据 | `audit_outbox::tests::audit_transport_rejects_remote_plaintext_and_url_credentials` |
| 重启恢复、收到确认前保留 | `audit_outbox::tests::pending_audit_survives_restart_until_acknowledged` |
| 失败同 nonce 补传、不发送旧 URL | `audit_outbox::tests::failed_delivery_is_retried_with_same_nonce_and_old_hosts_are_not_sent` |
| 不可写或非法事件不能假报落盘成功 | `audit_outbox::tests::invalid_or_unwritable_audit_is_not_reported_as_persisted` |
| 文件名称完整、分批 nonce 唯一 | `audit_outbox::tests::file_audit_batches_keep_all_names_and_unique_nonces` |
| 实际 HTTP 工厂不跟随重定向、不发送明文 | `audit_http_does_not_follow_redirects`、`remote_audit_factory_never_sends_plaintext` |

旧 10,000 次插入压测曾在前一轮依赖组合通过，本轮 SQLx 升级后未重跑该压测；新增临时数据库 CRUD / 重启测试用于本轮兼容性验证。基础模块和 HTTP 工厂测试不能替代完整客户端、Flutter、移动端、GUI 或生产联调验证。完整客户端 `cargo check --locked --lib --no-default-features` 已实际尝试；安装并验证 libclang 后继续构建，仍因未配置 `VCPKG_ROOT` / 原生媒体库而失败，不能宣称完整客户端类型检查通过。

## RustSec 扫描结果与剩余项

使用官方 rustsec 库 0.33.0，公告库固定为 `84d943a6deeb8cc6c6f632531aaf4f1c62275239`。计数是“依赖版本 × 公告”，包含非当前平台及开发/构建/锁文件可选依赖，不等于已证实可远程利用的漏洞数。

- 服务端：此前 31 → 上一轮 18 → 本轮 **4 条**。
- 客户端：此前 24 → 上一轮 12 → 本轮 **12 条**。
- 可选 Tauri UI 独立锁文件未在本轮修改或复核；不包含于上述服务端计数。

| 范围 | 尚存依赖与处理边界 |
| --- | --- |
| 服务端旧网络链（本轮已移除） | h2 0.3.26、rustls 0.21.12、rustls-webpki 0.101.7、webpki-roots 0.25.4、ring 0.16.20 已随 Reqwest 迁移从锁文件移除。 |
| 服务端锁文件可选数据库链 | rsa 0.9.10 / RUSTSEC-2023-0071 无公告补丁版本；SQLite 服务不启用 MySQL，但锁文件命中仍如实保留。 |
| 服务端其他平台/构建依赖 | quick-xml 0.39.4（2 条）、users 0.11.0；尚未全部替换。 |
| 客户端 | idna 0.5、libgit2-sys 0.14.2、quick-xml 0.30 / 0.31 / 0.37.5（共 6 条）、ringbuf 0.3.3、time 0.1.45 / 0.3.36、users 0.11。 |
| 客户端版本迁移限制 | time 修补需要提高 Rust 最低版本；部分依赖涉及旧 Sciter / 原生库 ABI，未通过改 CI 声明或盲升主版本掩盖兼容风险。 |

## 仍不能保证的安全属性

1. 不是企业 IAM / RBAC，审计共享 token 的持有者可以伪造设备事件；没有按设备签名、不可抵赖、独立防篡改归档或 SIEM 集成。
2. 异步生产者调度到落盘前仍有崩溃窗口；磁盘损坏/写满、管理员删除、强制断电的目录元数据持久性不能由文件同步完全解决。不是“任何时刻零丢失”或端到端 exactly-once 保证。
3. 服务端 SQLite / 客户端 outbox 没有自动清理未确认记录，避免为控制空间而静默丢证据。必须配置磁盘监控、访问控制、备份、归档及留存策略；去重窗口取决于保留的数据库记录。
4. 没有对公网容量、DoS、TLS 网关、证书更新、真实代理部署做压力/渗透测试。不要把本地测试作为对外开放许可。
5. 品牌、logo、安装包签名与全平台发布尚未在本轮实现；许可证和必要开源声明不能因换品牌被删除。
