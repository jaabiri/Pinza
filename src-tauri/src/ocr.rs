//! Reading the text in a screenshot, on the device: Tesseract on Linux, the
//! Vision framework on macOS, Windows.Media.Ocr on Windows.

use std::path::Path;

/// A word and where it is, in image pixels.
#[derive(Clone, Debug)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug, Default)]
pub struct TextLine {
    pub words: Vec<Word>,
}

impl TextLine {
    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }
}

pub fn text_of(lines: &[TextLine]) -> String {
    lines.iter().map(TextLine::text).filter(|t| !t.trim().is_empty()).collect::<Vec<_>>().join("\n")
}

/// Whether text can be read on this machine.
pub fn available() -> bool {
    #[cfg(target_os = "linux")]
    return crate::linux::has("tesseract");
    #[cfg(not(target_os = "linux"))]
    true
}

/// What to install when it cannot.
pub fn missing_hint() -> &'static str {
    "Install Tesseract to read text: sudo pacman -S tesseract tesseract-data-eng"
}

pub fn recognize(path: &Path, language: &str) -> Result<Vec<TextLine>, String> {
    imp::recognize(path, language)
}

/// Splits a line found as a whole into words, sharing out its box by
/// character position. Good enough to cover a word with a blur.
#[allow(dead_code)]
fn split_line(text: &str, x: f32, y: f32, w: f32, h: f32) -> TextLine {
    let total = text.chars().count().max(1) as f32;
    let mut words = Vec::new();
    let mut offset = 0usize;
    for part in text.split(' ') {
        let len = part.chars().count();
        if len > 0 {
            words.push(Word {
                text: part.to_string(),
                x: x + w * offset as f32 / total,
                y,
                w: w * len as f32 / total,
                h,
            });
        }
        offset += len + 1;
    }
    TextLine { words }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{TextLine, Word};
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::process::Command;

    /// `tesseract file stdout tsv`: one row per word, with its box and the
    /// block, paragraph and line it belongs to.
    pub fn recognize(path: &Path, language: &str) -> Result<Vec<TextLine>, String> {
        if !crate::linux::has("tesseract") {
            return Err(super::missing_hint().into());
        }
        let mut cmd = Command::new("tesseract");
        cmd.arg(path).arg("stdout");
        if !language.trim().is_empty() {
            cmd.args(["-l", language.trim()]);
        }
        let out = cmd.arg("tsv").output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(parse_tsv(&String::from_utf8_lossy(&out.stdout)))
    }

    pub fn parse_tsv(tsv: &str) -> Vec<TextLine> {
        let mut lines: BTreeMap<(u32, u32, u32, u32), TextLine> = BTreeMap::new();
        for row in tsv.lines().skip(1) {
            let c: Vec<&str> = row.split('\t').collect();
            if c.len() < 12 || c[0] != "5" {
                continue;
            }
            let text = c[11].trim();
            if text.is_empty() {
                continue;
            }
            let n = |i: usize| c[i].parse::<u32>().unwrap_or(0);
            let f = |i: usize| c[i].parse::<f32>().unwrap_or(0.0);
            lines.entry((n(1), n(2), n(3), n(4))).or_default().words.push(Word {
                text: text.to_string(),
                x: f(6),
                y: f(7),
                w: f(8),
                h: f(9),
            });
        }
        lines.into_values().collect()
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::TextLine;
    use std::path::Path;
    use std::process::Command;

    /// Vision through AppleScriptObjC: no extra install, nothing leaves the
    /// Mac. Prints one line per text observation with its normalised box.
    const SCRIPT: &str = r#"use AppleScript version "2.7"
use framework "Foundation"
use framework "Vision"
use scripting additions
on run argv
  set theURL to current application's |NSURL|'s fileURLWithPath:(item 1 of argv)
  set handler to current application's VNImageRequestHandler's alloc()'s initWithURL:theURL options:(current application's NSDictionary's dictionary())
  set request to current application's VNRecognizeTextRequest's alloc()'s init()
  request's setRecognitionLevel:(current application's VNRequestTextRecognitionLevelAccurate)
  request's setUsesLanguageCorrection:true
  set {ok, err} to handler's performRequests:(current application's NSArray's arrayWithObject:request) |error|:(reference)
  if ok is false then error (err's localizedDescription() as text)
  set out to {}
  repeat with observation in (request's results() as list)
    set candidate to (first item of ((observation's topCandidates:1) as list))
    set b to observation's boundingBox()
    -- Newer systems hand a CGRect over as a list, older ones as a record.
    try
      set {{bx, byy}, {bw, bh}} to b
    on error
      set bx to x of origin of b
      set byy to y of origin of b
      set bw to width of |size| of b
      set bh to height of |size| of b
    end try
    set end of out to (bx as text) & tab & (byy as text) & tab & (bw as text) & tab & (bh as text) & tab & ((candidate's |string|()) as text)
  end repeat
  set AppleScript's text item delimiters to linefeed
  return out as text
end run
"#;

    pub fn recognize(path: &Path, _language: &str) -> Result<Vec<TextLine>, String> {
        let (iw, ih) = image::image_dimensions(path).map_err(|e| e.to_string())?;
        let out = Command::new("osascript")
            .arg("-e")
            .arg(SCRIPT)
            .arg(path)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        let (iw, ih) = (iw as f32, ih as f32);
        let mut lines = Vec::new();
        for row in String::from_utf8_lossy(&out.stdout).lines() {
            let c: Vec<&str> = row.splitn(5, '\t').collect();
            if c.len() < 5 {
                continue;
            }
            // AppleScript may write decimals with a comma in some locales.
            let f = |s: &str| s.replace(',', ".").parse::<f32>().unwrap_or(0.0);
            let (x, y, w, h) = (f(c[0]), f(c[1]), f(c[2]), f(c[3]));
            // Vision measures from the bottom left, as a fraction of the image.
            lines.push(super::split_line(c[4], x * iw, (1.0 - y - h) * ih, w * iw, h * ih));
        }
        Ok(lines)
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use super::{TextLine, Word};
    use std::path::Path;
    use windows::core::HSTRING;
    use windows::Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::{FileAccessMode, StorageFile};

    pub fn recognize(path: &Path, _language: &str) -> Result<Vec<TextLine>, String> {
        run(path).map_err(|e| e.message().to_string())
    }

    fn run(path: &Path) -> windows::core::Result<Vec<TextLine>> {
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))?.join()?;
        let stream = file.OpenAsync(FileAccessMode::Read)?.join()?;
        let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
        let bitmap = decoder.GetSoftwareBitmapAsync()?.join()?;
        let bitmap = SoftwareBitmap::Convert(&bitmap, BitmapPixelFormat::Bgra8)?;
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
        let result = engine.RecognizeAsync(&bitmap)?.join()?;
        let mut lines = Vec::new();
        for line in result.Lines()? {
            let mut words = Vec::new();
            for word in line.Words()? {
                let r = word.BoundingRect()?;
                words.push(Word { text: word.Text()?.to_string(), x: r.X, y: r.Y, w: r.Width, h: r.Height });
            }
            lines.push(TextLine { words });
        }
        Ok(lines)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod imp {
    pub fn recognize(_: &std::path::Path, _: &str) -> Result<Vec<super::TextLine>, String> {
        Err("Reading text is not supported here".into())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn groups_words_into_lines() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   5\t1\t1\t1\t1\t1\t10\t10\t40\t12\t96\thello\n\
                   5\t1\t1\t1\t1\t2\t55\t10\t40\t12\t96\tworld\n\
                   5\t1\t1\t1\t2\t1\t10\t30\t80\t12\t96\tme@example.com\n";
        let lines = super::imp::parse_tsv(tsv);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text(), "hello world");
        assert_eq!(lines[1].words[0].x, 10.0);
    }

    #[test]
    fn splits_a_line_by_characters() {
        let line = super::split_line("ab cd", 0.0, 0.0, 50.0, 10.0);
        assert_eq!(line.words.len(), 2);
        assert_eq!(line.words[1].x, 30.0);
    }
}
