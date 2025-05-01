use std::{
    env, fs,
    io::{Cursor, Read},
    thread,
    time::Duration,
};

use flate2::read::ZlibDecoder;
use minifb::{Window, WindowOptions};

#[allow(dead_code)]
struct Chunk {
    //  A 4-byte unsigned integer giving the number of bytes in
    //  the chunk's data field.
    length: u32,
    // A 4-byte chunk type code.
    typ: [u8; 4],
    // The data bytes appropriate to the chunk type, if any.
    data: Vec<u8>,
    // A 4-byte CRC (Cyclic Redundancy Check) calculated on the
    // preceding bytes in the chunk, including the chunk type code and
    // chunk data fields, but not including the length field.
    crc: [u8; 4],
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct Ihdr {
    // give dimensions in pixels
    // zero is invalid dimension
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    compression_method: u8,
    filter_method: u8,
    interlace_method: u8,
}

struct Decoder {
    header: Option<Ihdr>,
    data: Vec<u8>,
    cursor: usize,
}

impl Decoder {
    fn new(path: &str) -> Self {
        let content = fs::read(path).unwrap();
        //The first eight bytes of a PNG file always contain the following
        // (decimal) values:
        // 137 80 78 71 13 10 26 10
        let sig = &content[0..8];
        assert_eq!(sig, [137, 80, 78, 71, 13, 10, 26, 10]);

        let data = content[8..].to_vec();
        Self {
            header: None,
            data,
            cursor: 0,
        }
    }

    fn next_chunk(&mut self) -> Option<Chunk> {
        if self.cursor + 12 > self.data.len() {
            return None;
        }

        let length =
            u32::from_be_bytes(self.data[self.cursor..self.cursor + 4].try_into().unwrap());

        let typ = self.data[self.cursor + 4..self.cursor + 8]
            .try_into()
            .unwrap();

        let data_start = self.cursor + 8;
        let data_end = data_start + length as usize;
        let chunk_data = self.data[data_start..data_end].to_vec();

        let crc = self.data[data_end..data_end + 4].try_into().unwrap();

        self.cursor = data_end + 4;

        Some(Chunk {
            length,
            typ,
            data: chunk_data,
            crc,
        })
    }

    fn parse_ihdr(&self, chunk: &[u8]) -> Ihdr {
        assert_eq!(chunk.len(), 13, "ihdr chunk size must be 13");

        let width = u32::from_be_bytes(chunk[..4].try_into().unwrap());
        let height = u32::from_be_bytes(chunk[4..8].try_into().unwrap());

        Ihdr {
            width,
            height,
            bit_depth: chunk[8],
            color_type: chunk[9],
            compression_method: chunk[10],
            filter_method: chunk[11],
            interlace_method: chunk[12],
        }
    }

    //  Deflate-compressed datastreams within PNG are stored in the "zlib"
    // format,
    fn decompress_data(&mut self) {
        let mut decompressed = Vec::new();
        let mut deflator = ZlibDecoder::new(Cursor::new(&self.data));
        deflator
            .read_to_end(&mut decompressed)
            .expect("Failed to decompress chunk!");
        self.data = decompressed;
    }

    fn unfilter_scanlines(&mut self) {
        let header = self.header.clone().unwrap();
        let bytes_per_pixel: usize = match header.color_type {
            0 => 1, // grayscale
            2 => 3, // RGB
            // 3 => each pixelle is a palette index
            4 => 2, // Grayscale + alpha
            6 => 4, // RGB + alpha
            _ => return,
        };

        let stride = header.width as usize * bytes_per_pixel;
        let mut result = Vec::with_capacity(header.height as usize * stride);
        let mut prev_scanline = vec![0u8; stride as usize];

        let mut i = 0;
        for _ in 0..header.height {
            //  each scanline is preceded by a filter type byte
            // that specifies the filter algorithm used for that scanline.
            let filter_type = &self.data[i];
            i += 1;

            let scanline = &self.data[i..i + stride];
            let mut unfiltered = vec![0u8; stride as usize];

            match filter_type {
                // None
                0 => unfiltered.copy_from_slice(scanline),
                //Sub
                // Sub(x) + Raw(x-bpp)
                1 => {
                    for x in 0..stride {
                        let left = if x >= bytes_per_pixel {
                            unfiltered[x - bytes_per_pixel]
                        } else {
                            0
                        };

                        unfiltered[x] = scanline[x].wrapping_add(left);
                    }
                }
                // Up
                // Up(x) = Raw(x) - Prior(x)
                2 => {
                    for x in 0..stride {
                        unfiltered[x] = scanline[x].wrapping_add(prev_scanline[x]);
                    }
                }
                // Average
                // Average(x) = Raw(x) - floor((Raw(x-bpp)+Prior(x))/2)
                3 => {
                    for x in 0..stride {
                        let left = if x >= bytes_per_pixel {
                            unfiltered[x - bytes_per_pixel]
                        } else {
                            0
                        };

                        let up = prev_scanline[x];
                        let avg = ((left as u16 + up as u16) / 2) as u8;

                        unfiltered[x] = scanline[x].wrapping_add(avg);
                    }
                }
                // Paeth
                // Paeth(x) = Raw(x) - PaethPredictor(Raw(x-bpp), Prior(x), Prior(x-bpp))
                4 => {
                    for x in 0..stride {
                        let a = if x >= bytes_per_pixel {
                            unfiltered[x - bytes_per_pixel]
                        } else {
                            0
                        };

                        let b = prev_scanline[x];

                        let c = if x >= bytes_per_pixel {
                            prev_scanline[x - bytes_per_pixel]
                        } else {
                            0
                        };

                        unfiltered[x] = scanline[x].wrapping_add(self.paeth_predicator(a, b, c));
                    }
                }

                _ => panic!("Undupported filter type!"),
            }

            result.extend_from_slice(&unfiltered);
            prev_scanline = unfiltered;
            i += stride;
        }

        self.data = result;
    }

    fn paeth_predicator(&self, a: u8, b: u8, c: u8) -> u8 {
        let a = a as i32;
        let b = b as i32;
        let c = c as i32;

        let p = a + b - c;
        let pa = (p - a).abs();
        let pb = (p - b).abs();
        let pc = (p - c).abs();

        if pa <= pb && pa <= pc {
            return a as u8;
        } else if pb <= pc {
            return b as u8;
        } else {
            return c as u8;
        }
    }

    fn pixels_to_u32(&self) -> Vec<u32> {
        let color_type = self.header.clone().unwrap().color_type;
        let mut buffer = Vec::with_capacity(
            self.data.len()
                / match color_type {
                    2 => 3,
                    6 => 4,
                    _ => panic!("Unsupported format!"),
                },
        );

        match color_type {
            2 => {
                for chunk in self.data.chunks(3) {
                    let r = chunk[0] as u32;
                    let g = chunk[1] as u32;
                    let b = chunk[2] as u32;

                    buffer.push((r << 16) | (g << 8) | b);
                }
            }
            6 => {
                //   ignore alpha
                for chunk in self.data.chunks(3) {
                    let r = chunk[0] as u32;
                    let g = chunk[1] as u32;
                    let b = chunk[2] as u32;

                    buffer.push((r << 16) | (g << 8) | b);
                }
            }
            _ => {}
        }
        buffer
    }

    fn display_image(&self, pixels: &[u32]) {
        let header = self.header.clone().unwrap();
        let width = header.width as usize;
        let height = header.height as usize;
        let mut window = Window::new("PNG Viewer", width, height, WindowOptions::default())
            .expect("Failed to open window");

        while window.is_open() && !window.is_key_down(minifb::Key::Escape) {
            window
                .update_with_buffer(pixels, width, height)
                .expect("Failed to update window");
        }

        // allow cleanup
        thread::sleep(Duration::from_millis(100));
    }
}

fn help() {
    println!("USAGE: pigo");
    println!("  pigo image.png");
}

fn main() {
    let mut args: Vec<String> = env::args().collect();
    args.remove(0);
    if args.len() < 1 {
        help();
        return;
    }

    for image in args {
        let mut decoder = Decoder::new(&image);
        let header = decoder.next_chunk().unwrap();
        decoder.header = Some(decoder.parse_ihdr(&header.data));

        let mut image_data = Vec::new();

        while let Some(chunk) = decoder.next_chunk().as_mut() {
            let typ = str::from_utf8(&chunk.typ).unwrap();

            match typ {
                "IHDR" => {
                    eprintln!("Two Ihdrs");
                    break;
                }
                "IDAT" => {
                    image_data.append(&mut chunk.data);
                }
                "IEND" => {
                    break;
                }
                other => panic!("UKNOWN chunk type: {}", other),
            }
        }

        decoder.data = image_data;
        decoder.decompress_data();

        // unfilter the data
        decoder.unfilter_scanlines();
        let buffer = decoder.pixels_to_u32();
        decoder.display_image(&buffer);
    }
}
