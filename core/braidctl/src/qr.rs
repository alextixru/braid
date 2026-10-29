//! QR личной ссылки: в терминал (полублоки, светлые модули — белые, чтобы
//! читалось на тёмном фоне) и в PNG (ист. 36). Строится по матрице модулей
//! `qrcode`, без зависимости от его рендереров.

use anyhow::Context as _;
use image::{ImageBuffer, Luma};
use qrcode::{Color, QrCode};
use std::path::Path;

struct Matrix {
    width: usize,
    dark: Vec<bool>,
}

impl Matrix {
    fn new(text: &str) -> anyhow::Result<Self> {
        let code = QrCode::new(text.as_bytes()).context("QR: данные не помещаются")?;
        let width = code.width();
        let dark = code
            .to_colors()
            .into_iter()
            .map(|c| c == Color::Dark)
            .collect();
        Ok(Self { width, dark })
    }

    /// Тёмный ли модуль с учётом тихой зоны вокруг.
    fn is_dark(&self, x: isize, y: isize) -> bool {
        if x < 0 || y < 0 || x >= self.width as isize || y >= self.width as isize {
            return false;
        }
        self.dark[y as usize * self.width + x as usize]
    }
}

/// Текстовый QR: две строки модулей на одну строку символов.
pub fn terminal(text: &str) -> anyhow::Result<String> {
    const QUIET: isize = 2;
    let m = Matrix::new(text)?;
    let size = m.width as isize + QUIET * 2;
    let mut out = String::new();
    let mut y = -QUIET;
    while y < m.width as isize + QUIET {
        for x in -QUIET..size - QUIET {
            let top_light = !m.is_dark(x, y);
            let bottom_light = !m.is_dark(x, y + 1);
            out.push(match (top_light, bottom_light) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        out.push('\n');
        y += 2;
    }
    Ok(out)
}

/// PNG: `scale` пикселей на модуль, тихая зона 4 модуля.
pub fn png(text: &str, path: &Path, scale: u32) -> anyhow::Result<()> {
    const QUIET: u32 = 4;
    let m = Matrix::new(text)?;
    let modules = m.width as u32 + QUIET * 2;
    let size = modules * scale.max(1);
    let image = ImageBuffer::<Luma<u8>, Vec<u8>>::from_fn(size, size, |px, py| {
        let x = (px / scale.max(1)) as isize - QUIET as isize;
        let y = (py / scale.max(1)) as isize - QUIET as isize;
        Luma([if m.is_dark(x, y) { 0 } else { 255 }])
    });
    image
        .save(path)
        .with_context(|| format!("записать {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_qr_is_square_with_quiet_zone() {
        let text = terminal("braid://abc").unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let width = lines[0].chars().count();
        assert!(lines.iter().all(|l| l.chars().count() == width));
        // Тихая зона: первая строка целиком светлая.
        assert!(lines[0].chars().all(|c| c == '█'));
        assert!(lines.len() * 2 >= width - 1, "две строки модулей на строку");
    }

    #[test]
    fn png_is_written_with_scale() {
        let dir = std::env::temp_dir().join(format!("braidctl-qr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("qr.png");
        png("braid://abc", &path, 4).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
