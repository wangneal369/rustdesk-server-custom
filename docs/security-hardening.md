# 安全修复与功能边界（2026-10-24）

本项目按用户确认的用途收敛为：远控核心安全、稳定性、定制化以及服务端运行日志。不建设平台账号、JWT、权限系统、后台管理面板或集中操作审计。此文不是“全部漏洞修复”或可直接上线的结论。

## 保留的安全修复

- hbbs / hbbr 默认拒绝空白 KEY，只有显式 `ALLOW_ANONYMOUS=Y` 才允许空 KEY。Compose、systemd hbbr 参数与 s6 镜像默认值继续采用共享密钥配置。
- KEY / 公钥匹配不是人员身份认证。原生注册/探测不是所有消息都携带准入凭据；不能据此宣称全量身份认证或封闭设备登记。客户端既有密码、人工确认、可选 2FA、白名单及会话权限检查保持不变。
- WebSocket 服务入口继续复用客户端已经采用的 tokio-tungstenite / tungstenite 0.29 修补分支，固定为 `c241b11b03de4881faf1ecd2fe6e867c92f3013b` / `efce47e5364f00a63302cf07ab1fb13ded478914`。共享库内部 0.26.2 依赖及两个共享子模块固定提交不变。
- WebSocket 握手超时 10 秒；默认不信任转发地址，只对 `TRUSTED_PROXIES` 列出的代理解析来源，转发链从右向左停止于首个不可信跳点。
- TCP / WebSocket 准入控制消息限 128 KiB，TCP 超大声明长度在读取正文前拒绝，WebSocket 单帧和分片后的完整消息均受限制。中继通过准入后恢复媒体报文上限；读取缓冲随实际数据增长，不根据远端声明一次性扩张。未宣称限制总连接数或解决全部 DoS。
- 前阶段 SQLx 0.8.6、libsqlite3-sys 0.30.1、Reqwest 0.12 及其余依赖修补继续保留。本次仅删除不再使用的 Axum 接收端依赖，不回退旧 SQLx / TLS / Hyper 依赖链。Reqwest 仍用于原有版本检查。

## 仅保留服务端运行日志

沿用 flexi_logger：`LOG_DIR` 默认 `logs`，`RUST_LOG` 默认 `info`；hbbs / hbbr 分别写日志，同时将 info 及以上输出到 stdout。每日或 10 MiB 轮转，保留 31 个历史文件（不保证 31 天）；配置在日志初始化前加载。KEY 原值不写日志。

保留启动、异常、连接及认证失败等原有日志。这些日志用于运行排障，不提供人员操作追责、文件操作审计或屏幕内容记录。日志访问权限、磁盘空间、备份和留存由部署方管理。

## 本次取消的功能及兼容边界

- 删除新增的 `/api/audit/conn`、`file`、`alarm` 接收服务及其后台任务，不再监听审计端口。`AUDIT_LISTEN` / `AUDIT_TOKEN` 不再具有作用。
- 不再创建或写入 `audit_event` 表；原有 peer 表、参数化 CRUD、SQLx 兼容性修补不变。**已存在的审计表不会被 DROP 或清空。**
- 删除客户端新增的 outbox、自动重试补传、审计专用 HTTP 工厂、审计 token 选项、文件日志分批及对应专用测试。
- 客户端共用审计 URL 出口固定为空；即使旧配置指定了 `api-server` 或自建服务，也不上传连接、文件、告警及会话备注审计。备注菜单按空地址隐藏，原生备注发送空地址直接返回，手写 Web bridge 的对应审计出口/发送也关闭；不新增 FFI 或大规模裁剪上游 UI。
- 磁盘上已有的 `.audit-outbox`、旧数据库和旧配置不自动删除；历史 outbox 不再扫描或补传。需要归档或清理时由维护者明确执行。本机按用户授权清理了经逐行确认的旧 10,000 行测试库 `test.sqlite3`；保留编译及 peer 存储使用的 `db_v2.sqlite3`。
- 上游其他可选管理 API / 登录界面未大范围裁剪，也没有在本项目实现相应账号后端；不将它们当作本项目已交付的账号功能。

## 回归范围与证据

```powershell
# 临时 --config 仅用于本机下载源；--offline 要求依赖已缓存
cargo test --locked --offline --all-targets -- --skip database::tests::test_insert
# 客户端基础库
cargo test --locked --offline -p base --all-targets -- --include-ignored
```

最终验证：服务端库 18、hbbr 17、集成 3，共 38 次测试执行通过（共享测试在不同目标重复执行）；客户端基础库 19 项通过；实际纯函数及其禁用出口回归独立编译后 1 项通过，不计为完整客户端构建。

| 需求 | 精确测试证据 |
| --- | --- |
| 移除审计建表而不删除历史证据，peer 更新重启后保持 | `database::tests::peer_updates_and_existing_evidence_survive_database_reopen` |
| 客户端配置过 API 仍不产生审计 URL | `common::tests::centralized_audit_stays_disabled_with_configured_api_servers`；本机单独编译实际函数和此测试，不等于完整客户端构建 |
| 默认拒绝空 KEY | `common::tests::service_admission_requires_key_or_explicit_anonymous_mode`、`relay_startup_rejects_an_empty_key_by_default` |
| 服务端运行日志落盘 | `relay_startup_redacts_the_key_and_writes_a_log_file` |
| 超大控制报文及 WebSocket 分片拒绝 | `common::tests::control_messages_reject_oversized_headers_before_reading_payloads`、`websocket::tests::oversized_websocket_control_messages_are_rejected` |
| 错误中继凭据拒绝 | `relay_server::tests::invalid_relay_credentials_never_enter_the_pairing_pool` |
| 正确凭据下原生、WebSocket 及两种混合中继仍可转发较大数据 | `relay_server::tests::relay_media_payloads_are_not_limited_to_admission_size` |
| 代理来源校验与握手超时 | `websocket::tests::untrusted_peers_cannot_spoof_forwarded_addresses`、`forwarded_chain_stops_at_the_first_untrusted_hop`、`stalled_websocket_handshakes_time_out` |

旧的固定文件名 10,000 次插入压测本次未重跑，排除项不能计为通过。完整客户端此前在安装 libclang 后仍因缺少 VCPKG_ROOT / 原生媒体库而构建失败；基础库及独立纯函数测试不能代替完整客户端、Flutter 或全平台联调；本机没有 Dart / Flutter SDK，UI 适配仅源码复查，Web JS 核心不在本仓库。

## 依赖扫描与剩余限制

官方 rustsec 库 0.33.0，公告库固定为 `84d943a6deeb8cc6c6f632531aaf4f1c62275239`。计数为“依赖版本 × 公告”，包括其他平台、开发/构建及锁文件可选依赖，不等于已证实可利用漏洞数。

- 本次复核服务端仍为 4 条：quick-xml 0.39.4（2 条）、users 0.11.0、可选 MySQL 链中的 rsa 0.9.10；本次未用降低版本处理依赖。
- 本次复核客户端仍为 12 条：idna 0.5、libgit2-sys 0.14.2、quick-xml 0.30 / 0.31 / 0.37.5（6 条）、ringbuf 0.3.3、time 0.1.45 / 0.3.36、users 0.11。部分修补涉及最低 Rust 版本或原生 ABI 迁移，尚未全部处理。
- 可选 Tauri UI 的独立锁文件未在本次修改或复核。
- 没有公网容量、压力、生产 TLS / 代理或渗透测试；不提供完整人员身份管理或操作审计。
- 品牌、logo、安装包签名和全平台发布仍未完成；必要许可证及开源声明继续保留。
