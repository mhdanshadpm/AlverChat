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

/// Boundary for platforms without a native integration yet, including mobile.
///
/// Windowing and notifications degrade to no-ops because the host runtime still owns
/// them; permission work that cannot be performed must fail loudly rather than report
/// a consent the user never gave.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_never_claims_a_permission_was_granted() {
        let platform = StubPlatform;
        for permission in [
            Permission::LocalNetwork,
            Permission::Camera,
            Permission::Microphone,
        ] {
            assert_eq!(platform.request_permission(permission.clone()), Ok(false));
            assert!(platform.open_system_settings(permission).is_err());
        }
    }

    #[test]
    fn stub_window_and_notification_calls_are_harmless() {
        let platform = StubPlatform;
        assert!(platform.set_dock_visible(false).is_ok());
        assert!(platform.notify("AlverChat", "peer is online").is_ok());
    }
}
