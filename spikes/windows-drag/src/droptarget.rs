use std::path::PathBuf;

use windows::core::{implement, Result};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::*;

use crate::app;

#[implement(IDropTarget)]
pub struct DropTarget {
    hwnd: HWND,
    /// Lets Explorer keep drawing its drag image while the cursor is over the ring.
    helper: Option<IDropTargetHelper>,
}

impl DropTarget {
    pub fn new(hwnd: HWND) -> Self {
        let helper = unsafe { CoCreateInstance(&CLSID_DragDropHelper, None, CLSCTX_INPROC_SERVER).ok() };
        DropTarget { hwnd, helper }
    }
}

unsafe fn files_from(obj: &IDataObject) -> Vec<PathBuf> {
    let fmt = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(mut medium) = obj.GetData(&fmt) else { return Vec::new() };
    let hdrop = HDROP(medium.u.hGlobal.0);
    let count = DragQueryFileW(hdrop, u32::MAX, None);
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let len = DragQueryFileW(hdrop, i, None) as usize;
        let mut buf = vec![0u16; len + 1];
        DragQueryFileW(hdrop, i, Some(&mut buf));
        out.push(PathBuf::from(String::from_utf16_lossy(&buf[..len])));
    }
    ReleaseStgMedium(&mut medium);
    out
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(&self, data: Option<&IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let files = data.map(|d| unsafe { files_from(d) }).unwrap_or_default();
        let e = app::on_drag_enter(files, (pt.x, pt.y));
        unsafe {
            *effect = e;
            if let (Some(h), Some(d)) = (&self.helper, data) {
                let _ = h.DragEnter(self.hwnd, d, &POINT { x: pt.x, y: pt.y }, e);
            }
        }
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let e = app::on_drag_over((pt.x, pt.y));
        unsafe {
            *effect = e;
            if let Some(h) = &self.helper {
                let _ = h.DragOver(&POINT { x: pt.x, y: pt.y }, e);
            }
        }
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        app::on_drag_leave();
        if let Some(h) = &self.helper {
            unsafe {
                let _ = h.DragLeave();
            }
        }
        Ok(())
    }

    fn Drop(&self, data: Option<&IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let files = data.map(|d| unsafe { files_from(d) }).unwrap_or_default();
        let e = app::on_drop(files, (pt.x, pt.y));
        unsafe {
            *effect = e;
            if let (Some(h), Some(d)) = (&self.helper, data) {
                let _ = h.Drop(d, &POINT { x: pt.x, y: pt.y }, e);
            }
        }
        Ok(())
    }
}
