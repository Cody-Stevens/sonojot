//! Paths saved outside this process must survive an AppImage mount going away.
use std::{io, path::PathBuf};

/// The outer AppImage, never the executable in its temporary squashfs mount.
/// `APPIMAGE` alone is not proof: AppImage apps (T3 Code, editors, terminals)
/// pass it on to everything they launch, so a pacman-installed SonoJot started
/// from one would otherwise take that other app's path as its own. Trust it only
/// while this executable runs from inside the AppImage's own mount (`APPDIR`).
pub fn appimage() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let exe = std::env::current_exe().ok()?;
        return appimage_path(std::env::var_os("APPIMAGE"), std::env::var_os("APPDIR"), &exe);
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(any(target_os = "linux", test))]
fn appimage_path(
    value: Option<std::ffi::OsString>,
    appdir: Option<std::ffi::OsString>,
    exe: &std::path::Path,
) -> Option<PathBuf> {
    let appdir = PathBuf::from(appdir?);
    if !appdir.is_absolute() {
        return None;
    }
    let appdir = std::fs::canonicalize(&appdir).unwrap_or(appdir);
    if !exe.starts_with(&appdir) {
        return None;
    }
    value
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
}

/// For login entries and compositor bindings. Workers use current_exe instead.
pub fn installed() -> io::Result<PathBuf> {
    appimage().map(Ok).unwrap_or_else(std::env::current_exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appimage_requires_an_existing_absolute_file() {
        let dir = std::env::temp_dir().join(format!("sonojot-path-{}", uuid::Uuid::new_v4()));
        let mount = dir.join("mount");
        std::fs::create_dir_all(mount.join("usr/bin")).unwrap();
        let exe = std::fs::canonicalize(&mount).unwrap().join("usr/bin/SonoJot");
        let appdir = || Some(mount.clone().into_os_string());
        let path = dir.join("SonoJot's nightly.AppImage");
        std::fs::write(&path, b"fixture").unwrap();
        assert_eq!(appimage_path(Some(path.clone().into_os_string()), appdir(), &exe), Some(path.clone()));
        assert_eq!(appimage_path(None, appdir(), &exe), None);
        assert_eq!(appimage_path(Some("".into()), appdir(), &exe), None);
        assert_eq!(appimage_path(Some("relative.AppImage".into()), appdir(), &exe), None);
        assert_eq!(appimage_path(Some(dir.clone().into_os_string()), appdir(), &exe), None);
        assert_eq!(appimage_path(Some(dir.join("missing").into()), appdir(), &exe), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_inherited_appimage_from_another_app_is_ignored() {
        let dir = std::env::temp_dir().join(format!("sonojot-path-{}", uuid::Uuid::new_v4()));
        let other_mount = dir.join(".mount_T3-Code");
        std::fs::create_dir_all(&other_mount).unwrap();
        let other = dir.join("T3-Code.AppImage");
        std::fs::write(&other, b"fixture").unwrap();
        // SonoJot from pacman, started from inside another AppImage app.
        let exe = std::path::Path::new("/usr/bin/SonoJot");
        assert_eq!(appimage_path(Some(other.clone().into_os_string()), Some(other_mount.into_os_string()), exe), None);
        assert_eq!(appimage_path(Some(other.into_os_string()), None, exe), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
