//! The Windows folder picker, and opening a folder in Explorer.

/// Asks the user for a folder with Windows' own dialog. `None` when they
/// cancel it, or when the dialog could not be shown.
///
/// Blocks the calling thread while the dialog is open, which on the UI thread
/// is what a modal dialog is.
pub fn pick() -> Option<String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
    };

    unsafe {
        // Already initialised on this thread is fine; so is a different
        // apartment, in which case the dialog still works in the one it has.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        // Cancelling comes back as an error, which is the `None` it means.
        dialog.Show(HWND::default()).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok();
        CoTaskMemFree(Some(name.0 as *const _));
        path
    }
}

/// Opens `path` in Explorer, creating it first so the button never opens
/// nothing.
pub fn open(path: &std::path::Path) {
    let _ = std::fs::create_dir_all(path);
    crate::update::open_in_browser(&path.display().to_string());
}
