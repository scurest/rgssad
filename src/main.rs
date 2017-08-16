use std::fs;
use std::fs::File;
use std::io::SeekFrom;
use std::io::Seek;
use std::io::Read;
use std::io::Write;
use std::io::Take;
use std::io::Error;
use std::io::ErrorKind;
use std::collections::HashMap;
use std::env;
use std::cmp;
use std::path::Path;

extern crate regex;
use regex::Regex;

static __VERSION__: &str = "0.1.2";

fn advance_magic(magic: &mut u32) -> u32 {
    let old = *magic;
    *magic = magic.wrapping_mul(7) + 3;
    return old;
}

fn ru32(stream: &mut File, result: &mut u32) -> bool {
    let mut buff = [0; 4];
    if let Err(_) = stream.read_exact(&mut buff) {
        return false;
    }

    *result = (((buff[0] as u32) << 0x00) & 0x000000FF) |
              (((buff[1] as u32) << 0x08) & 0x0000FF00) |
              (((buff[2] as u32) << 0x10) & 0x00FF0000) |
              (((buff[3] as u32) << 0x18) & 0xFF000000) ;

    return true;
}

struct EntryData {
    offset: u32,
    magic: u32,
    size: u32,
}

struct Entry {
    offset: u32,
    magic: u32,
    stream: Take<File>,
}

impl Entry {
    fn read(&mut self, buf: &mut [u8]) -> usize {
        let mut maski = self.offset % 4;
        let mut offset = 0;
        let count = self.stream.read(buf).unwrap();
        let pre = ((4 - maski) % 4) as usize;

        self.offset += count as u32;

        for _ in 0..cmp::min(pre, count) {
            buf[offset] ^= ((self.magic >> (maski * 8)) & 0xff) as u8;
            maski += 1; offset += 1;
            if maski % 4 == 0 {
                advance_magic(&mut self.magic);
                maski = 0;
            }
        }

        if maski != 0 { return count; }

        unsafe {
            let len = (count - pre) / 4;
            let dat = buf[..len*4].as_mut_ptr() as *mut u32;

            for i in 0..(len as isize) {
                *dat.offset(i) = *dat.offset(i) ^ advance_magic(&mut self.magic);
            }

            offset += len * 4;
        }

        for i in 0..(count%4) {
            buf[offset + i] ^= ((self.magic >> (maski * 8)) & 0xff) as u8;
        }

        return count;
    }
}

struct RGSSArchive {
    entry: HashMap<String, EntryData>,
    stream: File,
}

impl RGSSArchive {
    // fn create(location: &str, version: u8) -> Result<RGSSArchive, Error> {
    //     let stream = File::create(location)?;
    //     if version < 1 || version > 3 {
    //         return Err(Error::new(ErrorKind::InvalidData, "Invalid version."));
    //     }
    //
    //     stream.write_all(&[b'R', b'G', b'S', b'S', b'A', b'D', version]);
    //
    //     Ok(RGSSArchive { entry: HashMap::new(), stream: stream })
    // }

    fn open(location: &str) -> Result<RGSSArchive, Error> {
        let mut stream = File::open(location)?;

        let mut header = [0u8; 8];
        stream.read_exact(&mut header)?;

        match String::from_utf8(header[..6].to_vec()) {
            Ok(h) => {
                if h != "RGSSAD" {
                    return Err(Error::new(ErrorKind::InvalidData, "Input file header mismatch."));
                }
            },
            Err(_) => return Err(Error::new(ErrorKind::InvalidData, "Input file header mismatch."))
        }

        // Check rgssad file version.
        return match header[7] {
            1|2 => RGSSArchive::open_rgssad(stream),
              3 => RGSSArchive::open_rgss3a(stream),
              _ => Err(Error::new(ErrorKind::InvalidData, format!("RGSSArchive file version:{} is not support.", header[7]))),
        }
    }

    fn open_rgssad(mut stream: File) -> Result<RGSSArchive, Error> {
        let mut magic = 0xDEADCAFEu32;
        let mut entry = HashMap::new();

        loop {
            let mut name_len: u32 = 0;
            if !ru32(&mut stream, &mut name_len) { break }
            name_len ^= advance_magic(&mut magic);

            let mut name_buf = vec![0u8; name_len as usize];
            stream.read_exact(&mut name_buf)?;
            for i in 0..(name_len as usize) {
                name_buf[i] ^= (advance_magic(&mut magic) & 0xff) as u8;
                if name_buf[i] == '\\' as u8 { name_buf[i] = '/' as u8 }
            }
            let name_buf = String::from_utf8(name_buf);
            if let Err(_) = name_buf { break }
            let name_buf = name_buf.unwrap();

            let mut data = EntryData { size: 0, offset: 0, magic: 0 };
            ru32(&mut stream, &mut data.size);
            data.size ^= advance_magic(&mut magic);
            data.offset = stream.seek(SeekFrom::Current(0))? as u32;
            data.magic = magic;

            stream.seek(SeekFrom::Current(data.size as i64))?;
            entry.insert(name_buf, data);
        }

        stream.seek(SeekFrom::Start(0))?;
        return Ok(RGSSArchive { entry: entry, stream: stream });
    }

    fn open_rgss3a(mut stream: File) -> Result<RGSSArchive, Error> {
        let mut magic = 0u32;
        let mut entry = HashMap::new();

        if !ru32(&mut stream, &mut magic) {
            return Err(Error::new(ErrorKind::InvalidData, format!("Magic number read failed.")));
        }
        magic = magic * 9 + 3;

        loop {
            let mut offset: u32 = 0;
            let mut size: u32 = 0;
            let mut start_magic: u32 = 0;
            let mut name_len: u32 = 0;

            if !ru32(&mut stream, &mut offset) { break };
            offset ^= magic;

            if offset == 0 { break }

            if !ru32(&mut stream, &mut size) { break }
            size ^= magic;

            if !ru32(&mut stream, &mut start_magic) { break}
            start_magic ^= magic;

            if !ru32(&mut stream, &mut name_len) { break }
            name_len ^= magic;

            let mut name_buf = vec![0u8; name_len as usize];
            stream.read_exact(&mut name_buf)?;
            for i in 0..(name_len as usize) {
                name_buf[i] ^= ((magic >> 8*(i%4)) & 0xff) as u8;
                if name_buf[i] == '\\' as u8 { name_buf[i] = '/' as u8 }
            }
            let name_buf = String::from_utf8(name_buf);
            if let Err(_) = name_buf { break }
            let name_buf = name_buf.unwrap();

            let data = EntryData {
                size: size, offset: offset, magic: start_magic
            };

            entry.insert(name_buf, data);
        }

        stream.seek(SeekFrom::Start(0))?;
        return Ok(RGSSArchive { entry: entry, stream: stream });
    }

    fn read_entry(&self, key: &str) -> Result<Entry, Error> {
        match self.entry.get(key) {
            Some(entry) => {
                let mut stream = self.stream.try_clone()?;
                stream.seek(SeekFrom::Start(entry.offset as u64))?;
                Ok(Entry {
                    offset: 0,
                    magic: entry.magic,
                    stream: stream.take(entry.size as u64),
                })
            }
            None => Err(Error::new(ErrorKind::InvalidData, "Key not found.")),
        }
    }
}

fn usage() {
    println!("Extract rgssad/rgss2a/rgss3a files.
Commands:
    help
    version
    list        file
    unpack      file output [filter]");
}

fn create(location: String) -> File {
    let path = Path::new(location.as_str());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    return File::create(path.to_str().unwrap()).unwrap();
}

fn list(archive: RGSSArchive) {
    for (name, data) in archive.entry {
        println!("{}: EntryData {{ size: {}, offset: {}, magic: {} }}", name, data.size, data.offset, data.magic);
    }
}

// fn pack(dir, out: &str, version: u8) -> io::Result<()> {
//     let dir = Path::new(dir);
//     if !dir.is_dir() {
//         println!("FAILED: input is not a dir."); return;
//     }
//     let archive = RGSSArchive::create(out, version);
//     if let Err(err) = archive {
//         println!("FAILED: {}", err.to_string()); return;
//     }
//     let archive = archive.unwrap();
//
//     let visit = |d: &Path| {
//         for entry in fs::read_dir(&dir)? {
//             let entry = entry?;
//             let path = entry.path();
//             if path.is_dir() {
//                 visit(&path);
//             } else {
//
//             }
//         }
//     }
// }

fn unpack(archive: RGSSArchive, dir: &str, filter: &str) {
    let entries = archive.entry.iter();
    let filter = match Regex::new(filter) {
        Ok(re) => re,
        Err(_) => Regex::new("*").unwrap(),
    };

    let mut buf = [0u8; 8192];

    for (name, _) in entries {
        if !filter.is_match(name) { continue }

        println!("Extracting: {}", name);
        let entry = archive.read_entry(name);
        if let Err(err) = entry {
            println!("FAILED: read entry failed, {}", err.to_string()); return;
        }
        let mut entry = entry.unwrap();

        let mut file = create(dir.to_string() + &"/".to_string() + &name.to_string());
        loop {
            let count = entry.read(&mut buf);
            if count == 0 { break }
            if let Err(err) = file.write(&buf[..count]) {
                println!("FAILED: key save failed, {}", err.to_string()); return;
            }
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 { usage(); return }
    match args[1].as_str() {
        "help" => usage(),
        "version" => {
            assert!(args.len() == 2);
            println!("version: {}", __VERSION__);
        },
        "list" => {
            assert!(args.len() == 3);
            let archive = RGSSArchive::open(args[2].as_str());
            if let Err(err) = archive {
                println!("FAILED: file parse failed, {}", err.to_string()); return;
            }
            let archive = archive.unwrap();

            list(archive);
        },
        "unpack" => {
            assert!(args.len() > 3 && args.len() < 6);
            let archive = RGSSArchive::open(args[2].as_str());
            if let Err(err) = archive {
                println!("FAILED: file parse failed, {}", err.to_string()); return;
            }
            let archive = archive.unwrap();
            unpack(archive, args[3].as_str(), (if args.len() == 5 { args[4].as_str() } else { "*" }));
        },
        _ => usage(),
    }
}
