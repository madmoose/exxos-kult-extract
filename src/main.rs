use std::fs::{self, File};
use std::io::{BufWriter, Read};
use std::path::Path;

pub trait ReadBytesExt: std::io::Read {
    #[inline]
    fn read_u8(&mut self) -> Result<u8, std::io::Error> {
        let mut buf = [0; 1];
        self.read_exact(&mut buf)?;
        Ok(buf[0])
    }
}

impl<R: std::io::Read> ReadBytesExt for R {}

pub trait WriteBytesExt: std::io::Write {
    #[inline]
    fn write_u8(&mut self, v: u8) -> Result<(), std::io::Error> {
        let buf = v.to_le_bytes();
        self.write_all(&buf)
    }
}

impl<W: std::io::Write> WriteBytesExt for W {}

// Based on https://int10h.org/blog/2022/06/ibm-5153-color-true-cga-palette/
// Index 0 has been changed to transparent
// Index 8 has been changed to black.
const EGA_PAL: [[u8; 4]; 16] = [
    [0x00, 0x00, 0x00, 0x00], //  0
    [0x00, 0x00, 0xc4, 0xff], //  1
    [0x00, 0xc4, 0x00, 0xff], //  2
    [0x00, 0xc4, 0xc4, 0xff], //  3
    [0xc4, 0x00, 0x00, 0xff], //  4
    [0xc4, 0x00, 0xc4, 0xff], //  5
    [0xc4, 0x7e, 0x00, 0xff], //  6
    [0xc4, 0xc4, 0xc4, 0xff], //  7
    [0x00, 0x00, 0x00, 0xff], //  8
    [0x4e, 0x4e, 0xdc, 0xff], //  9
    [0x4e, 0xdc, 0x4e, 0xff], // 10
    [0x4e, 0xf3, 0xf3, 0xff], // 11
    [0xdc, 0x4e, 0x4e, 0xff], // 12
    [0xf3, 0x4e, 0xf3, 0xff], // 13
    [0xf3, 0xf3, 0x4e, 0xff], // 14
    [0xff, 0xff, 0xff, 0xff], // 15
];

#[allow(clippy::erasing_op, clippy::identity_op)]
fn decode_planar_ega_to_rgba(src: &[u8], width: usize, height: usize) -> Vec<u8> {
    const PLANE_SIZE: usize = 8000;

    let mut frame = vec![0u8; width * height * 4];

    for y in 0..height {
        for x in 0..width {
            let ofs = width * y + x;
            let bitofs = 7 - x % 8;

            let p0 = (src[0 * PLANE_SIZE + ofs / 8] >> bitofs) & 1;
            let p1 = (src[1 * PLANE_SIZE + ofs / 8] >> bitofs) & 1;
            let p2 = (src[2 * PLANE_SIZE + ofs / 8] >> bitofs) & 1;
            let p3 = (src[3 * PLANE_SIZE + ofs / 8] >> bitofs) & 1;

            let v = (p3 << 3) | (p2 << 2) | (p1 << 1) | p0;

            for c in 0..4 {
                frame[4 * (y * width + x) + c] = EGA_PAL[v as usize][c];
            }
        }
    }

    frame
}

fn decode_interleaved_ega_to_rgba(src: &[u8], span: usize, height: usize) -> Vec<u8> {
    let width = 2 * span;
    let mut frame = vec![0u8; width * height * 4];

    for y in 0..height {
        for x in 0..width {
            let ofs = y * span + x / 2;
            let b = src[ofs];
            let v = if x % 2 == 0 { b >> 4 } else { b & 0x0f };

            for c in 0..4 {
                frame[4 * (y * width + x) + c] = EGA_PAL[v as usize][c];
            }
        }
    }

    frame
}

fn write_rgba_to_png<P: AsRef<Path>>(
    filename: P,
    data: &[u8],
    width: usize,
    height: usize,
) -> Result<(), std::io::Error> {
    const SCALE_FACTOR_WIDTH: usize = 5;
    const SCALE_FACTOR_HEIGHT: usize = 6;

    let file = File::create(filename)?;
    let w = BufWriter::new(file);

    let scaled_width = SCALE_FACTOR_WIDTH * width;
    let scaled_height = SCALE_FACTOR_HEIGHT * height;

    let mut encoder = png::Encoder::new(w, scaled_width as u32, scaled_height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);

    let mut writer = encoder.write_header()?;

    let scaled_size = 5 * width * 6 * height;
    let mut scaled_data = vec![0; 4 * scaled_size];

    for y in 0..height {
        for dy in 0..SCALE_FACTOR_HEIGHT {
            for x in 0..width {
                let ofs = y * width + x;
                for dx in 0..SCALE_FACTOR_WIDTH {
                    let sy = SCALE_FACTOR_HEIGHT * y + dy;
                    let sx = SCALE_FACTOR_WIDTH * x + dx;

                    let scaled_ofs = sy * scaled_width + sx;

                    for c in 0..4 {
                        scaled_data[4 * scaled_ofs + c] = data[4 * ofs + c];
                    }
                }
            }
        }
    }

    writer.write_image_data(&scaled_data)?;

    Ok(())
}

fn extract_fullscreen_ega<P: AsRef<Path>>(
    src: Vec<u8>,
    input_filename: P,
) -> Result<(), std::io::Error> {
    let width = 320;
    let height = 200;

    let frame_rgb = decode_planar_ega_to_rgba(&src, width, height);

    let filename = input_filename.as_ref();
    let output_filename = format!(
        "png/{}.png",
        filename.file_stem().unwrap().to_str().unwrap(),
    );

    write_rgba_to_png(output_filename, &frame_rgb, width, height)?;

    Ok(())
}

fn extract_sprites_ega<P: AsRef<Path>>(
    src: Vec<u8>,
    input_filename: P,
) -> Result<(), std::io::Error> {
    if src.len() < 4 {
        println!("Not a valid sprite sheet, file too small.");
        return Ok(());
    }

    let size = u32::from_be_bytes(src[0..4].try_into().unwrap()) as usize;

    if size + 4 != src.len() {
        println!("Not a valid sprite sheet, size in header incorrect.");
        return Ok(());
    }

    let mut src = &src[4..];

    let mut n = 0;
    while !src.is_empty() {
        let input_size = u16::from_le_bytes(src[0..2].try_into().unwrap()) as usize;
        let byte_width = 2 * src[2] as usize;
        let height = src[3] as usize;

        let width = 2 * byte_width;

        let frame_rgb = decode_interleaved_ega_to_rgba(&src[4..], byte_width, height);

        let filename = input_filename.as_ref();
        let output_filename = format!(
            "png/{}-{:02}.png",
            filename.file_stem().unwrap().to_str().unwrap(),
            n
        );
        n += 1;

        write_rgba_to_png(&output_filename, &frame_rgb, width, height)?;

        src = &src[input_size..];
    }

    Ok(())
}

fn main() -> Result<(), std::io::Error> {
    if std::env::args().len() <= 1 {
        let name = std::env::args().next().unwrap_or_default();
        println!("\nUsage: {name} path/to/kult/*.ega\n");
        println!("Will create a folder called `png` in which the output images is placed.\n");
        println!("The extracted PNGs will be scaled 5x in width and 6x in height.\n");
        return Ok(());
    }

    fs::create_dir_all("png")?;

    for filename in std::env::args().skip(1) {
        println!("Extracting {}", filename);

        let mut src = Vec::new();
        File::open(&filename)?.read_to_end(&mut src)?;

        if src.len() == 32000 {
            extract_fullscreen_ega(src, filename)?;
        } else {
            extract_sprites_ega(src, filename)?;
        }
    }

    Ok(())
}
