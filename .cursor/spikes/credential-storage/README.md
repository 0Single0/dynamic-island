# 凭据存储 Spike

日期：2026-09-29  
关联计划：`PLAN-00-03`  
实现接口：`src-tauri/src/agent/secrets.rs`

## 结论

Windows 主方案选择 Windows Credential Manager（Windows 凭据管理器），Rust 通过 `keyring` 的 Windows native backend 读写；业务代码只依赖 `SecretStore` trait。DPAPI（Data Protection API，数据保护 API）不作为首版主存储：它适合保护应用自己管理的字节数据，但还要自行解决密文文件的权限、原子更新、迁移和备份；此处 Windows 已提供按当前用户隔离的凭据保管库。

凭据引用采用 `provider/{provider_id}/{secret_kind}`，只作为凭据条目的账号名，不包含 secret。Provider 数据库行仅保存引用。读取结果在 Rust 内部以 `Zeroizing<String>` 包装，错误文本采用固定描述，避免把底层请求、秘密或输入值拼入日志。

## Spike 范围

- 增加可注入的 `SecretStore` 接口和 Windows 系统凭据后端包装。
- 用内存 fake 测试不存在、写入、读取、删除和引用校验；测试不向当前 Windows 用户凭据库写入任何条目。
- DPAPI 保留为未来需要加密任意本地 blob 时的备选，不提供明文文件回退。
- 本 Spike 关闭“API Key 是否明文落盘”的设计问题，但不替代真实应用中的 Windows 安装包、用户切换、错误恢复和密钥生命周期验收。

## 安全约束

- API 不得返回 secret 明文给 WebView；Provider 请求构造只在 Rust 内取用。
- 不记录 secret、请求 Authorization Header、Keyring 错误中的输入值；诊断只记录操作类别和脱敏错误码。
- 轮换时先写入新条目，再更新 Provider 引用，最后删除旧条目；删除 Provider 时清理相关引用。
- 所有凭据操作应按 provider ID 做隔离，不允许模型提供任意 keyring target。

## 官方参考

- [Windows Credential Manager: CredWriteW](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew)
- [Windows Credential Manager: CredReadW](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credreadw)
- [DPAPI: CryptProtectData](https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata)
- [Rust keyring crate](https://docs.rs/keyring/latest/keyring/)
