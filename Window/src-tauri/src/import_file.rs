//! The only path is returned by the native, user-confirmed selector, never by IPC input.
use crate::{
    error::{AppError, AppResult},
    learning_format::MAX_JSON_BYTES,
};
use serde::Serialize;
use std::{fs::File, io::Read, path::Path};

#[derive(Serialize)]
pub struct SelectedImportFile {
    pub name: String,
    pub content: String,
}
fn failure() -> AppError {
    AppError::new(
        "FileReadFailure",
        "선택한 자료 파일을 읽지 못했어요. 파일과 접근 권한을 확인해주세요.",
        true,
    )
}
fn read_selected(path: &Path) -> AppResult<SelectedImportFile> {
    let file = File::open(path).map_err(|_| failure())?;
    let meta = file.metadata().map_err(|_| failure())?;
    if !meta.is_file() || meta.len() > MAX_JSON_BYTES as u64 {
        return Err(AppError::new(
            "ValidationError",
            "파일은 각각 16 MiB 이하로 준비해주세요.",
            false,
        ));
    }
    let mut bytes = Vec::new();
    file.take((MAX_JSON_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| failure())?;
    if bytes.len() > MAX_JSON_BYTES {
        return Err(AppError::validation("file_size"));
    }
    let content = String::from_utf8(bytes).map_err(|_| {
        AppError::new(
            "ValidationError",
            "UTF-8로 저장한 JSON 파일을 선택해주세요.",
            false,
        )
    })?;
    let content = content
        .strip_prefix('\u{feff}')
        .unwrap_or(&content)
        .to_owned();
    Ok(SelectedImportFile {
        name: path
            .file_name()
            .ok_or_else(failure)?
            .to_string_lossy()
            .into_owned(),
        content,
    })
}
#[cfg(windows)]
pub fn select(owner: isize, kind: &str) -> AppResult<Option<SelectedImportFile>> {
    use windows::{
        core::w,
        Win32::{
            Foundation::HWND,
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{
                Common::COMDLG_FILTERSPEC, FileOpenDialog, IFileOpenDialog, FOS_DONTADDTORECENT,
                FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, SIGDN_FILESYSPATH,
            },
        },
    };
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }
    // Dedicated STA thread, owned modal dialog; all COM objects drop before Apartment.
    let path = unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|_| failure())?;
        let _apartment = Apartment;
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(|_| failure())?;
        dialog
            .SetOptions(
                FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST | FOS_DONTADDTORECENT,
            )
            .map_err(|_| failure())?;
        dialog
            .SetFileTypes(&[COMDLG_FILTERSPEC {
                pszName: w!("JSON 자료"),
                pszSpec: w!("*.json"),
            }])
            .map_err(|_| failure())?;
        dialog
            .SetTitle(if kind == "learning" {
                w!("학습 자료 선택")
            } else {
                w!("보충 자료 선택")
            })
            .map_err(|_| failure())?;
        if let Err(error) = dialog.Show(Some(HWND(owner as *mut std::ffi::c_void))) {
            if error.code().0 == 0x800704c7u32 as i32 {
                return Ok(None);
            }
            return Err(failure());
        }
        let item = dialog.GetResult().map_err(|_| failure())?;
        let raw = item
            .GetDisplayName(SIGDN_FILESYSPATH)
            .map_err(|_| failure())?;
        let path = raw.to_string();
        CoTaskMemFree(Some(raw.0 as *const std::ffi::c_void));
        path.map_err(|_| failure())?
    };
    read_selected(Path::new(&path)).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_bom_is_removed_but_json_and_answer_characters_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("synthetic.json");
        let text = "{\"synthetic\":\"가상  문자열\\r\\n\"}\r\n";
        std::fs::write(&path, format!("\u{feff}{text}")).unwrap();
        assert_eq!(read_selected(&path).unwrap().content, text);
    }
    #[test]
    fn oversized_invalid_utf8_and_missing_selected_files_fail_without_exposing_paths() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("synthetic.json");
        let file = File::create(&path).unwrap();
        file.set_len((MAX_JSON_BYTES + 1) as u64).unwrap();
        drop(file);
        assert!(read_selected(&path).is_err());
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(read_selected(&path).is_err());
        assert!(read_selected(&root.path().join("missing.json")).is_err());
    }
}
