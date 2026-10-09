fn main() {
    tauri_build::build();
    // 测试二进制不会经过 tauri-build 的主程序清单嵌入，但 tauri-plugin-dialog
    // （rfd 的 common-controls-v6 特性）静态导入 TaskDialogIndirect；
    // 没有 Common-Controls v6 激活上下文时加载器找不到该入口点（0xC0000139）。
    let manifest = std::path::Path::new(&std::env::var("OUT_DIR").unwrap())
        .join("common-controls-v6.manifest");
    std::fs::write(
        &manifest,
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*" />
    </dependentAssembly>
  </dependency>
</assembly>
"#,
    )
    .unwrap();
    println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
        manifest.display()
    );
}
