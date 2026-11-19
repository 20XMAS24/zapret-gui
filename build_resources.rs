//! Minimal COFF resource object writer. Keeps GNU builds independent of windres/Windows SDK.
use std::{collections::BTreeMap, path::Path};

fn u16_at(data: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(data[at..at + 2].try_into().unwrap())
}
fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().unwrap())
}
fn put16(data: &mut [u8], at: usize, value: u16) {
    data[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn reserve(data: &mut Vec<u8>, count: usize) -> usize {
    let at = data.len();
    data.resize(at + count, 0);
    at
}
fn directory(data: &mut Vec<u8>, count: usize) -> usize {
    let at = reserve(data, 16 + 8 * count);
    put16(data, at + 14, count.try_into().unwrap());
    at
}
fn entry(data: &mut [u8], at: usize, id: u32, offset: usize, is_dir: bool) {
    put32(data, at, id);
    put32(data, at + 4, offset as u32 | if is_dir { 0x80000000 } else { 0 });
}

pub fn embed(icon: &Path, manifest: Option<String>) {
    let ico = std::fs::read(icon).expect("read application icon");
    assert_eq!(&ico[..4], &[0, 0, 1, 0], "invalid ICO header");
    let count = u16_at(&ico, 4) as usize;
    let mut resources: BTreeMap<u32, Vec<(u32, Vec<u8>)>> = BTreeMap::new();
    let mut group = ico[..6].to_vec();
    for i in 0..count {
        let at = 6 + i * 16;
        let size = u32_at(&ico, at + 8) as usize;
        let offset = u32_at(&ico, at + 12) as usize;
        resources.entry(3).or_default().push(((i + 1) as u32, ico[offset..offset + size].to_vec()));
        group.extend_from_slice(&ico[at..at + 12]);
        group.extend_from_slice(&((i + 1) as u16).to_le_bytes());
    }
    resources.insert(14, vec![(1, group)]);
    if let Some(xml) = manifest {
        resources.insert(24, vec![(1, xml.into_bytes())]);
    }

    let mut data = Vec::new();
    let root = directory(&mut data, resources.len());
    let mut leaves = Vec::new();
    for (i, (kind, values)) in resources.into_iter().enumerate() {
        let names = directory(&mut data, values.len());
        entry(&mut data, root + 16 + i * 8, kind, names, true);
        for (j, (id, bytes)) in values.into_iter().enumerate() {
            let languages = directory(&mut data, 1);
            entry(&mut data, names + 16 + j * 8, id, languages, true);
            let leaf = reserve(&mut data, 16);
            entry(&mut data, languages + 16, 0, leaf, false);
            leaves.push((leaf, bytes));
        }
    }
    let mut relocations = Vec::new();
    for (leaf, bytes) in leaves {
        while data.len() % 4 != 0 { data.push(0); }
        let offset = data.len();
        put32(&mut data, leaf, offset as u32);
        put32(&mut data, leaf + 4, bytes.len() as u32);
        relocations.push(leaf as u32);
        data.extend(bytes);
    }
    let (machine, relocation): (u16, u16) = match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "x86_64" => (0x8664, 3),
        "x86" => (0x014c, 7),
        "aarch64" => (0xaa64, 2),
        arch => panic!("unsupported Windows architecture: {arch}"),
    };
    let raw_size = data.len() as u32;
    let symbols = 60 + raw_size + relocations.len() as u32 * 10;
    let mut object = vec![0; 60];
    put16(&mut object, 0, machine);
    put16(&mut object, 2, 1);
    put32(&mut object, 8, symbols);
    put32(&mut object, 12, 1);
    object[20..28].copy_from_slice(b".rsrc\0\0\0");
    put32(&mut object, 36, raw_size);
    put32(&mut object, 40, 60);
    put32(&mut object, 44, 60 + raw_size);
    put16(&mut object, 52, relocations.len().try_into().unwrap());
    put32(&mut object, 56, 0x40300040);
    object.extend(data);
    for offset in relocations {
        object.extend(offset.to_le_bytes());
        object.extend(0u32.to_le_bytes()); // section symbol index
        object.extend(relocation.to_le_bytes());
    }
    object.extend(b".rsrc\0\0\0");
    object.extend(0u32.to_le_bytes());
    object.extend(1u16.to_le_bytes()); // section 1
    object.extend([0, 0, 3, 0]); // static symbol, no auxiliary symbols
    object.extend(4u32.to_le_bytes()); // empty string table, including its length
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("app-resources.o");
    std::fs::write(&out, object).expect("write icon/manifest resource object");
    println!("cargo:rustc-link-arg={}", out.display());
}
