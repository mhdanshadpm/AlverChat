//! Native integration boundary. OS-specific implementations belong here only.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Permission {
    LocalNetwork,
    Camera,
    Microphone,
}

pub trait PlatformServices: Send + Sync {
    fn set_dock_visible(&self, visible: bool) -> Result<(), String>;
    fn notify(&self, title: &str, body: &str) -> Result<(), String>;
    fn request_permission(&self, permission: Permission) -> Result<bool, String>;
    fn open_system_settings(&self, permission: Permission) -> Result<(), String>;
}

pub struct StubPlatform;
impl PlatformServices for StubPlatform {
    fn set_dock_visible(&self, _: bool) -> Result<(), String> {
        Ok(())
    }
    fn notify(&self, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn request_permission(&self, _: Permission) -> Result<bool, String> {
        Ok(false)
    }
    fn open_system_settings(&self, _: Permission) -> Result<(), String> {
        Err("Native settings integration is not configured for this target".into())
    }
}
