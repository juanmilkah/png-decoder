// Png Decoder

use std::{
    fs,
    io::{Cursor, Read},
};

use flate2::read::ZlibDecoder;

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
        Self { data, cursor: 0 }
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
    fn decompress(&self, data: &[u8]) -> Vec<u8> {
        let mut decompressed = Vec::new();
        let mut deflator = ZlibDecoder::new(Cursor::new(data));
        deflator
            .read_to_end(&mut decompressed)
            .expect("Failed to decompress chunk!");
        decompressed
    }
}

fn main() {
    let mut decoder = Decoder::new("image.png");
    let header = decoder.next_chunk().unwrap();
    let header = decoder.parse_ihdr(&header.data);
    println!("Dimensions: {}x{}", header.height, header.width);

    let mut image_data = Vec::new();

    while let Some(chunk) = decoder.next_chunk().as_mut() {
        let typ = str::from_utf8(&chunk.typ).unwrap();

        match typ {
            "IHDR" => {
                eprintln!("Two Ihdrs");
                break;
            }
            "IDAT" => {
                println!("Data chunk with length: {}", chunk.length);
                image_data.append(&mut chunk.data);
            }
            "IEND" => {
                println!("End Chunk");
                break;
            }
            other => panic!("UKNOWN chunk type: {}", other),
        }
    }

    let decompressed = decoder.decompress(&image_data);
    println!("IMAGE DATA: {} BYTES", decompressed.len());
}
