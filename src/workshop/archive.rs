//! Exact-byte archive construction for native Workshop source snapshots.
use std::collections::BTreeMap;

/// The same store-only ZIP envelope as the browser exporter, including its
/// central directory. Bytes are never decoded or normalized here.
pub fn store_zip(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut central = Vec::new();
    let count = u16::try_from(files.len()).map_err(|_| "Too many archive members")?;
    for (path, data) in files {
        let name = path.as_bytes();
        let name_len = u16::try_from(name.len()).map_err(|_| "Archive path too long")?;
        let size = u32::try_from(data.len()).map_err(|_| "Archive member too large")?;
        let offset = u32::try_from(bytes.len()).map_err(|_| "Archive too large")?;
        let crc = crate::world::mod_pack::crc32(data);
        let mut local = vec![0; 30];
        local[0..4].copy_from_slice(&0x0403_4b50u32.to_le_bytes());
        local[4..6].copy_from_slice(&20u16.to_le_bytes());
        local[12..14].copy_from_slice(&0x21u16.to_le_bytes());
        local[14..18].copy_from_slice(&crc.to_le_bytes());
        local[18..22].copy_from_slice(&size.to_le_bytes());
        local[22..26].copy_from_slice(&size.to_le_bytes());
        local[26..28].copy_from_slice(&name_len.to_le_bytes());
        bytes.extend(local);
        bytes.extend(name);
        bytes.extend(data);
        let mut entry = vec![0; 46];
        entry[0..4].copy_from_slice(&0x0201_4b50u32.to_le_bytes());
        entry[4..6].copy_from_slice(&20u16.to_le_bytes());
        entry[6..8].copy_from_slice(&20u16.to_le_bytes());
        entry[14..16].copy_from_slice(&0x21u16.to_le_bytes());
        entry[16..20].copy_from_slice(&crc.to_le_bytes());
        entry[20..24].copy_from_slice(&size.to_le_bytes());
        entry[24..28].copy_from_slice(&size.to_le_bytes());
        entry[28..30].copy_from_slice(&name_len.to_le_bytes());
        entry[42..46].copy_from_slice(&offset.to_le_bytes());
        central.extend(entry);
        central.extend(name);
    }
    let offset = u32::try_from(bytes.len()).map_err(|_| "Archive too large")?;
    let size = u32::try_from(central.len()).map_err(|_| "Archive too large")?;
    bytes.extend(central);
    let mut end = vec![0; 22];
    end[0..4].copy_from_slice(&0x0605_4b50u32.to_le_bytes());
    end[8..10].copy_from_slice(&count.to_le_bytes());
    end[10..12].copy_from_slice(&count.to_le_bytes());
    end[12..16].copy_from_slice(&size.to_le_bytes());
    end[16..20].copy_from_slice(&offset.to_le_bytes());
    bytes.extend(end);
    Ok(bytes)
}
