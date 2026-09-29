# 文件编码与路径安全 Spike

日期：2026-09-29  
关联计划：`PLAN-00-10`、GAP-07、GAP-15  
实现：`src-tauri/src/agent/filesystem.rs`

## 结论

- Agent 文件工具先接受用户通过选择器明确授权的目录/文件，并把授权根交给 Rust `PathPolicy`；模型不能直接扩大根目录。
- 读取时 canonicalize（规范化真实路径）并检查授权根；写入时 canonicalize 父目录，再拼接文件名，避免目标文件不存在时无法 canonicalize。
- 默认拒绝 UNC（Universal Naming Convention，通用命名约定）网络共享路径、路径越权和符号链接链；Junction（目录联接）在 Windows 上按重解析点继续加固，并在真实实现中补充 `FILE_ATTRIBUTE_REPARSE_POINT` 检查。
- 正式写入必须再叠加外部修改检测、临时文件同目录写入、原子替换、备份、撤销和用户确认；本 Spike 只关闭路径解析边界。

## 测试

离线单元测试覆盖已授权文件、授权目录下的新输出文件、授权根外路径和 UNC 拒绝。
