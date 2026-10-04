use windows::Win32::Foundation::{CloseHandle, HANDLE};

mod autostart;
mod desktop;
pub(super) mod foreground;
mod instance;
mod locations;

pub(super) use autostart::{launch_at_login_enabled, set_launch_at_login};
pub(super) use desktop::{edit_file, open_directory};
pub(super) use instance::{InstanceGuard, claim_single_instance};
pub(super) use locations::default_data_root;

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
