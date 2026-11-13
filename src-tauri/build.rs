fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Tauri's resource is linked to the app binary, but not to Rust unit-test harnesses or examples.
        // Give non-app targets their own manifest so tests and examples can resolve TaskDialogIndirect.
        let out_dir = std::env::var_os("OUT_DIR").ok_or_else(|| std::io::Error::other("OUT_DIR is not set"))?;
        let manifest_path = std::path::PathBuf::from(out_dir).join("windows-test-manifest.xml");
        std::fs::write(
            &manifest_path,
            r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
</assembly>
"#,
        )?;
        let manifest_display = manifest_path.display();
        if std::env::var_os("CARGO_FEATURE_CLIPPY").is_some() {
            println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
            println!("cargo:rustc-link-arg=/MANIFESTINPUT:{manifest_display}");
        } else {
            println!("cargo:rustc-link-arg-examples=/MANIFEST:EMBED");
            println!("cargo:rustc-link-arg-examples=/MANIFESTINPUT:{manifest_display}");
        }
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let source = std::path::Path::new("native/hearthstone-overlay");
        println!("cargo:rerun-if-changed={}", source.display());
        let arch = std::env::var("CARGO_CFG_TARGET_ARCH")?;
        let target = if arch == "aarch64" {
            "arm64-apple-macosx11.0"
        } else {
            "x86_64-apple-macosx11.0"
        };
        let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR not set")?);
        let status = std::process::Command::new("xcrun")
            .args(["swiftc", "-O", "-parse-as-library", "-target", target])
            .arg(source.join("main.swift"))
            .arg(source.join("GameSceneMonitor.swift"))
            .arg(source.join("GameplayVisibility.swift"))
            .arg("-o")
            .arg(output.join("hearthstone-overlay"))
            .status()?;
        if !status.success() {
            return Err("Failed to compile Hearthstone overlay".into());
        }
    }

    #[cfg(feature = "clippy")]
    {
        println!("cargo:warning=Skipping tauri_build during Clippy");
    }

    #[cfg(not(feature = "clippy"))]
    tauri_build::build();

    Ok(())
}
