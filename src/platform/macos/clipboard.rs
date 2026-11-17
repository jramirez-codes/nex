//! Copied files and images on the clipboard, read with a JavaScript for Automation script
//! (`osascript`), which reaches `NSPasteboard` without an AppKit binding here.

use std::path::PathBuf;
use std::time::Duration;

use crate::platform::ClipboardFiles;
use crate::platform::unix::output_within;

/// Prints `files` and one path per line, or writes a PNG to `argv[0]` and prints `png`
/// (converting a TIFF, which some apps copy instead), or prints nothing.
const SCRIPT: &str = r#"
ObjC.import('AppKit');
function run(argv) {
  const pb = $.NSPasteboard.generalPasteboard;
  const urls = pb.readObjectsForClassesOptions($([$.NSURL]), $({ NSPasteboardURLReadingFileURLsOnlyKey: true }));
  if (urls && !urls.isNil() && urls.count > 0) {
    const out = [];
    for (let i = 0; i < urls.count; i++) out.push(urls.objectAtIndex(i).path.js);
    return 'files\n' + out.join('\n');
  }
  let data = pb.dataForType('public.png');
  if (data.isNil()) {
    const tiff = pb.dataForType('public.tiff');
    if (tiff.isNil()) return '';
    data = $.NSBitmapImageRep.imageRepWithData(tiff).representationUsingTypeProperties(4, $({}));
    if (data.isNil()) return '';
  }
  return data.writeToFileAtomically(argv[0], true) ? 'png' : '';
}
"#;

pub fn clipboard_files() -> Option<ClipboardFiles> {
    let png = std::env::temp_dir().join(format!("nex-clipboard-{}.png", std::process::id()));
    let out = output_within("osascript", &["-l", "JavaScript", "-e", SCRIPT, &png.to_string_lossy()], Duration::from_secs(5))?;
    let out = String::from_utf8_lossy(&out);
    let mut lines = out.lines();
    match lines.next()?.trim() {
        "files" => {
            let paths: Vec<PathBuf> = lines.filter(|l| !l.is_empty()).map(PathBuf::from).collect();
            (!paths.is_empty()).then_some(ClipboardFiles::Paths(paths))
        }
        "png" => {
            let bytes = std::fs::read(&png).ok();
            let _ = std::fs::remove_file(&png);
            bytes.filter(|b| !b.is_empty()).map(ClipboardFiles::Png)
        }
        _ => None,
    }
}
