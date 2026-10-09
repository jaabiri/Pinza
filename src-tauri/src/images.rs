//! New pictures made from the ones on the line: a redacted copy, and
//! several joined into one.

use crate::line::unique_path;
use crate::ocr::{self, TextLine};
use image::imageops::{self, FilterType};
use image::{DynamicImage, GenericImageView, RgbaImage};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Things nobody should share by accident.
fn patterns() -> &'static [Regex] {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        [
            // Email addresses.
            r"[\w.+-]+@[\w-]+(\.[\w-]+)+",
            // Phone, card and account numbers: long runs of digits.
            r"\+?\d[\d \-().]{7,}\d",
            // IP addresses.
            r"\b\d{1,3}(\.\d{1,3}){3}\b",
            // Links that carry a query or a token.
            r"https?://\S*[?#=]\S*",
            // Well-known API key and token shapes.
            r"\b(sk|pk|rk)_(live|test)_[A-Za-z0-9]{8,}",
            r"\b(ghp|gho|ghu|ghs|github_pat)_[A-Za-z0-9_]{16,}",
            r"\bxox[abpors]-[A-Za-z0-9-]{10,}",
            r"\bAKIA[0-9A-Z]{16}\b",
            r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
            // Long random-looking strings.
            r"\b[A-Za-z0-9_\-]{32,}\b",
            // IBANs.
            r"\b[A-Z]{2}\d{2}(?: ?[A-Z0-9]{4}){3,7}\b",
        ]
        .iter()
        .map(|p| Regex::new(p).expect("valid pattern"))
        .collect()
    })
}

/// Areas of the picture that hold something sensitive, in pixels.
pub fn sensitive_areas(lines: &[TextLine]) -> Vec<(f32, f32, f32, f32)> {
    let mut areas = Vec::new();
    for line in lines {
        // Rebuild the line's text and remember where each word sits in it.
        let mut text = String::new();
        let mut spans = Vec::new();
        for (i, w) in line.words.iter().enumerate() {
            if i > 0 {
                text.push(' ');
            }
            let start = text.len();
            text.push_str(&w.text);
            spans.push((start, text.len()));
        }
        let mut hit = vec![false; line.words.len()];
        for re in patterns() {
            for m in re.find_iter(&text) {
                for (i, (s, e)) in spans.iter().enumerate() {
                    if *s < m.end() && *e > m.start() {
                        hit[i] = true;
                    }
                }
            }
        }
        // Neighbouring hits become one area, so a phone number is one blur.
        let mut i = 0;
        while i < hit.len() {
            if !hit[i] {
                i += 1;
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            while i < hit.len() && hit[i] {
                let w = &line.words[i];
                x0 = x0.min(w.x);
                y0 = y0.min(w.y);
                x1 = x1.max(w.x + w.w);
                y1 = y1.max(w.y + w.h);
                i += 1;
            }
            areas.push((x0, y0, x1 - x0, y1 - y0));
        }
    }
    areas
}

/// Pixelates the areas so they cannot be read back.
pub fn pixelate(img: &mut RgbaImage, areas: &[(f32, f32, f32, f32)]) {
    let (iw, ih) = img.dimensions();
    for &(x, y, w, h) in areas {
        let pad = (h * 0.25).max(3.0);
        let x0 = (x - pad).max(0.0) as u32;
        let y0 = (y - pad).max(0.0) as u32;
        let x1 = ((x + w + pad).ceil() as u32).min(iw);
        let y1 = ((y + h + pad).ceil() as u32).min(ih);
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let (rw, rh) = (x1 - x0, y1 - y0);
        let block = (rh / 2).clamp(6, 24);
        let region = imageops::crop_imm(img, x0, y0, rw, rh).to_image();
        let small = imageops::resize(&region, (rw / block).max(1), (rh / block).max(1), FilterType::Triangle);
        let big = imageops::resize(&small, rw, rh, FilterType::Nearest);
        imageops::replace(img, &big, x0 as i64, y0 as i64);
    }
}

/// Saves `name (redacted).png` next to the original. Returns the new file
/// and how many areas were covered; nothing is written when none were found.
pub fn redact(path: &Path, language: &str) -> Result<(Option<PathBuf>, usize), String> {
    let lines = ocr::recognize(path, language)?;
    let areas = sensitive_areas(&lines);
    if areas.is_empty() {
        return Ok((None, 0));
    }
    let mut img = image::open(path).map_err(|e| e.to_string())?.into_rgba8();
    pixelate(&mut img, &areas);
    let dir = path.parent().ok_or("no folder")?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let out = unique_path(dir, &format!("{stem} (redacted)"), "png");
    img.save_with_format(&out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok((Some(out), areas.len()))
}

/// Joins pictures into one, top to bottom or side by side, scaled to the
/// narrowest (or shortest) so nothing is blown up.
pub fn join(paths: &[PathBuf], vertical: bool) -> Result<PathBuf, String> {
    let images: Vec<DynamicImage> =
        paths.iter().map(|p| image::open(p).map_err(|e| format!("{}: {e}", p.display()))).collect::<Result<_, _>>()?;
    if images.len() < 2 {
        return Err("Pick two or more pictures".into());
    }
    const GAP: u32 = 16;
    let side = if vertical {
        images.iter().map(|i| i.width()).min().unwrap()
    } else {
        images.iter().map(|i| i.height()).min().unwrap()
    };
    let scaled: Vec<RgbaImage> = images
        .iter()
        .map(|img| {
            let (w, h) = img.dimensions();
            let (nw, nh) = if vertical {
                (side, (h as f64 * side as f64 / w as f64).round().max(1.0) as u32)
            } else {
                ((w as f64 * side as f64 / h as f64).round().max(1.0) as u32, side)
            };
            if (nw, nh) == (w, h) {
                img.to_rgba8()
            } else {
                img.resize_exact(nw, nh, FilterType::Lanczos3).to_rgba8()
            }
        })
        .collect();
    let gaps = GAP * (scaled.len() as u32 - 1);
    let (cw, ch) = if vertical {
        (side, scaled.iter().map(|i| i.height()).sum::<u32>() + gaps)
    } else {
        (scaled.iter().map(|i| i.width()).sum::<u32>() + gaps, side)
    };
    let mut canvas = RgbaImage::new(cw, ch);
    let mut offset = 0i64;
    for img in &scaled {
        if vertical {
            imageops::replace(&mut canvas, img, 0, offset);
            offset += (img.height() + GAP) as i64;
        } else {
            imageops::replace(&mut canvas, img, offset, 0);
            offset += (img.width() + GAP) as i64;
        }
    }
    let dir = paths[0].parent().ok_or("no folder")?;
    let out = unique_path(dir, "Pinza stack", "png");
    canvas.save_with_format(&out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::{TextLine, Word};

    fn line(words: &[(&str, f32)]) -> TextLine {
        TextLine {
            words: words.iter().map(|(t, x)| Word { text: t.to_string(), x: *x, y: 10.0, w: 40.0, h: 12.0 }).collect(),
        }
    }

    #[test]
    fn finds_emails_and_phone_numbers() {
        let l = line(&[("Mail", 0.0), ("me@example.com", 50.0), ("or", 100.0), ("+34", 150.0), ("600", 200.0), ("123", 250.0), ("456", 300.0)]);
        let areas = sensitive_areas(&[l]);
        assert_eq!(areas.len(), 2);
        assert_eq!(areas[0].0, 50.0);
        // The phone number spans four words but is one area.
        assert_eq!(areas[1].0, 150.0);
        assert_eq!(areas[1].2, 190.0);
    }

    #[test]
    fn leaves_plain_text_alone() {
        assert!(sensitive_areas(&[line(&[("Quarterly", 0.0), ("report", 50.0)])]).is_empty());
    }

    #[test]
    fn finds_tokens() {
        let l = line(&[("token:", 0.0), ("ghp_abcdefghijklmnopqrstuvwxyz0123", 50.0)]);
        assert_eq!(sensitive_areas(&[l]).len(), 1);
    }
}
