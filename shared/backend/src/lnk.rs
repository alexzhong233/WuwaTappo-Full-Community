//! Pure Rust Windows `.lnk` (MS-SHLLINK) shortcut writer.
//!
//! Replaces `windows/tools/make_shortcut.py` and `pylnk3` without requiring Python
//! or COM (`WScript.Shell`, which is blocked by security policy on some machines).

use std::{
    fs,
    path::{Path, PathBuf},
};

const LINK_CLSID: [u8; 16] = [
    0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
];

const CLSID_MY_COMPUTER: [u8; 16] = [
    0xE0, 0x4F, 0xD0, 0x20, 0xEA, 0x3A, 0x69, 0x10, 0xA2, 0xD8, 0x08, 0x00, 0x2B, 0x30, 0x30, 0x9D,
];

const FLAG_HAS_LINK_TARGET_ID_LIST: u32 = 0x0000_0001;
const FLAG_HAS_LINK_INFO: u32 = 0x0000_0002;
const FLAG_HAS_NAME: u32 = 0x0000_0004;
const FLAG_HAS_WORKING_DIR: u32 = 0x0000_0010;
const FLAG_HAS_ARGUMENTS: u32 = 0x0000_0020;
const FLAG_HAS_ICON_LOCATION: u32 = 0x0000_0040;
const FLAG_IS_UNICODE: u32 = 0x0000_0080;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowMode {
    Normal = 1,
    Maximized = 3,
    Minimized = 7,
}

impl WindowMode {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "normal" => Ok(Self::Normal),
            "maximized" => Ok(Self::Maximized),
            "minimized" => Ok(Self::Minimized),
            other => Err(format!("未知窗口模式：{other}（期望 normal/maximized/minimized）")),
        }
    }
}

pub struct ShortcutSpec<'a> {
    pub out: &'a Path,
    pub target: &'a str,
    pub arguments: Option<&'a str>,
    pub workdir: Option<&'a str>,
    pub icon: Option<&'a str>,
    pub description: Option<&'a str>,
    pub window: WindowMode,
}

fn push_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_utf16le_nul(buf: &mut Vec<u8>, text: &str) {
    for u in text.encode_utf16().chain(Some(0)) {
        push_u16(buf, u);
    }
}

fn push_string_data(buf: &mut Vec<u8>, text: &str) {
    let units: Vec<u16> = text.encode_utf16().collect();
    push_u16(buf, units.len().min(u16::MAX as usize) as u16);
    for u in units {
        push_u16(buf, u);
    }
}

fn build_file_entry_id(name: &str, is_dir: bool) -> Vec<u8> {
    let mut item = Vec::new();
    // Size placeholder
    push_u16(&mut item, 0);
    // Type: 0x31 dir, 0x32 file
    item.push(if is_dir { 0x31 } else { 0x32 });
    item.push(0); // unknown/sort byte
    push_u32(&mut item, 0); // file size
    push_u16(&mut item, 0); // modified date
    push_u16(&mut item, 0); // modified time
    push_u16(&mut item, if is_dir { 0x10 } else { 0x20 }); // attributes
    // ASCII short name (null-terminated, padded to even)
    let short_name: Vec<u8> = name
        .bytes()
        .map(|b| if b.is_ascii_graphic() { b } else { b'_' })
        .take(64)
        .collect();
    item.extend_from_slice(&short_name);
    item.push(0);
    if item.len() % 2 != 0 {
        item.push(0);
    }

    // BEEF0004 Vista+ Unicode filename extra block inside ItemID
    let ext_start = item.len();
    push_u16(&mut item, 0); // ext size placeholder
    push_u16(&mut item, 0x0009); // version
    push_u32(&mut item, 0xBEEF_0004);
    push_u16(&mut item, 0); // cdate
    push_u16(&mut item, 0); // ctime
    push_u16(&mut item, 0); // adate
    push_u16(&mut item, 0); // atime
    push_u32(&mut item, 0x002E); // unknown / version offset
    push_u16(&mut item, 0); // unknown
    push_u32(&mut item, 0); // NTFS file ref lo
    push_u32(&mut item, 0); // NTFS file ref hi
    push_u64_zero(&mut item); // unknown (8 bytes)
    push_u16(&mut item, 0); // long string size (optional 0)
    push_utf16le_nul(&mut item, name); // primary UTF-16LE name
    let first_off = (ext_start - 0) as u16; // offset to BEEF0004 from item start = 2? Store +42
    push_u16(&mut item, first_off);

    let ext_len = (item.len() - ext_start) as u16;
    item[ext_start..ext_start + 2].copy_from_slice(&ext_len.to_le_bytes());
    let total_len = item.len() as u16;
    item[0..2].copy_from_slice(&total_len.to_le_bytes());
    item
}

fn push_u64_zero(buf: &mut Vec<u8>) {
    buf.extend_from_slice(&[0u8; 8]);
}

fn split_win_path(raw: &str) -> (String, Vec<String>) {
    let normalized = raw.replace('/', "\\");
    let bytes = normalized.as_bytes();
    if bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'\\' {
        let drive = format!("{}:\\", normalized.chars().next().unwrap().to_ascii_uppercase());
        let rest: Vec<String> = normalized[3..]
            .split('\\')
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        (drive, rest)
    } else {
        let rest: Vec<String> = normalized
            .trim_start_matches('\\')
            .split('\\')
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        ("C:\\".into(), rest)
    }
}

fn build_idlist(target: &str) -> Vec<u8> {
    let (drive, components) = split_win_path(target);
    let mut idlist = Vec::new();

    // 1. Root folder: My Computer (20 bytes)
    let mut root = Vec::new();
    push_u16(&mut root, 20);
    root.push(0x1F);
    root.push(0x50);
    root.extend_from_slice(&CLSID_MY_COMPUTER);
    idlist.extend_from_slice(&root);

    // 2. Drive item: e.g. C:\ (25 bytes)
    let mut drv = Vec::new();
    push_u16(&mut drv, 25);
    drv.push(0x2F);
    drv.extend_from_slice(drive.as_bytes());
    while drv.len() < 25 {
        drv.push(0);
    }
    idlist.extend_from_slice(&drv);

    // 3. Path segments
    for (idx, part) in components.iter().enumerate() {
        let is_dir = idx + 1 < components.len();
        idlist.extend_from_slice(&build_file_entry_id(part, is_dir));
    }

    // TerminalID
    push_u16(&mut idlist, 0);

    let mut out = Vec::with_capacity(idlist.len() + 2);
    push_u16(&mut out, idlist.len() as u16);
    out.extend_from_slice(&idlist);
    out
}

fn build_link_info(target: &str) -> Vec<u8> {
    // LinkInfo with Unicode header (0x24 = 36 bytes) + VolumeID + LocalBasePath + CommonPathSuffix + LocalBasePathUnicode + CommonPathSuffixUnicode
    let mut vol = Vec::new();
    let label_offset = 16u32;
    push_u32(&mut vol, 17); // VolumeIDSize
    push_u32(&mut vol, 3); // DRIVE_FIXED
    push_u32(&mut vol, 0); // DriveSerialNumber
    push_u32(&mut vol, label_offset);
    vol.push(0); // Empty ANSI VolumeLabel

    let ascii_path: Vec<u8> = target
        .bytes()
        .map(|b| if b.is_ascii() { b } else { b'?' })
        .chain(Some(0))
        .collect();

    let hdr_size = 36usize;
    let vol_off = hdr_size;
    let local_base_off = vol_off + vol.len();
    let common_suffix_off = local_base_off + ascii_path.len();
    let local_base_uni_off = common_suffix_off + 1;
    let mut unicode_path = Vec::new();
    push_utf16le_nul(&mut unicode_path, target);
    let common_suffix_uni_off = local_base_uni_off + unicode_path.len();
    let total_size = common_suffix_uni_off + 2;

    let mut buf = Vec::with_capacity(total_size);
    push_u32(&mut buf, total_size as u32);
    push_u32(&mut buf, hdr_size as u32);
    push_u32(&mut buf, 1); // VolumeIDAndLocalBasePath
    push_u32(&mut buf, vol_off as u32);
    push_u32(&mut buf, local_base_off as u32);
    push_u32(&mut buf, 0); // CommonNetworkRelativeLinkOffset
    push_u32(&mut buf, common_suffix_off as u32);
    push_u32(&mut buf, local_base_uni_off as u32);
    push_u32(&mut buf, common_suffix_uni_off as u32);
    buf.extend_from_slice(&vol);
    buf.extend_from_slice(&ascii_path);
    buf.push(0); // CommonPathSuffix ANSI ""
    buf.extend_from_slice(&unicode_path);
    push_u16(&mut buf, 0); // CommonPathSuffixUnicode ""
    buf
}

pub fn encode_shortcut(spec: &ShortcutSpec<'_>) -> Vec<u8> {
    let mut flags = FLAG_HAS_LINK_TARGET_ID_LIST | FLAG_HAS_LINK_INFO | FLAG_IS_UNICODE;
    if spec.description.is_some() {
        flags |= FLAG_HAS_NAME;
    }
    if spec.workdir.is_some() {
        flags |= FLAG_HAS_WORKING_DIR;
    }
    if spec.arguments.is_some() {
        flags |= FLAG_HAS_ARGUMENTS;
    }
    if spec.icon.is_some() {
        flags |= FLAG_HAS_ICON_LOCATION;
    }

    let mut out = Vec::new();
    // ShellLinkHeader (76 bytes)
    push_u32(&mut out, 0x4C);
    out.extend_from_slice(&LINK_CLSID);
    push_u32(&mut out, flags);
    push_u32(&mut out, 0x20); // FILE_ATTRIBUTE_ARCHIVE
    push_u64_zero(&mut out); // CreationTime
    push_u64_zero(&mut out); // AccessTime
    push_u64_zero(&mut out); // WriteTime
    push_u32(&mut out, 0); // FileSize
    push_u32(&mut out, 0); // IconIndex
    push_u32(&mut out, spec.window as u32);
    push_u16(&mut out, 0); // HotKey
    push_u16(&mut out, 0); // Reserved1
    push_u32(&mut out, 0); // Reserved2
    push_u32(&mut out, 0); // Reserved3
    debug_assert_eq!(out.len(), 76);

    out.extend_from_slice(&build_idlist(spec.target));
    out.extend_from_slice(&build_link_info(spec.target));

    if let Some(desc) = spec.description {
        push_string_data(&mut out, desc);
    }
    if let Some(workdir) = spec.workdir {
        push_string_data(&mut out, workdir);
    }
    if let Some(args) = spec.arguments {
        push_string_data(&mut out, args);
    }
    if let Some(icon) = spec.icon {
        push_string_data(&mut out, icon);
    }

    // ExtraData TerminalBlock
    push_u32(&mut out, 0);
    out
}

pub fn create_shortcut(spec: &ShortcutSpec<'_>, verify_exists: bool) -> Result<usize, String> {
    if verify_exists {
        for (label, opt) in [
            ("target", Some(spec.target)),
            ("workdir", spec.workdir),
            ("icon", spec.icon),
        ] {
            if let Some(p) = opt {
                if !Path::new(p).exists() {
                    return Err(format!("{label} 不存在：{p}"));
                }
            }
        }
    }
    let abs_out = if spec.out.is_absolute() {
        spec.out.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(spec.out)
    };
    if let Some(parent) = abs_out.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if abs_out.exists() {
        let _ = fs::remove_file(&abs_out);
    }
    let bytes = encode_shortcut(spec);
    fs::write(&abs_out, &bytes).map_err(|e| format!("写入快捷方式失败：{e}"))?;
    Ok(bytes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16_at(data: &[u8], off: usize) -> u16 {
        u16::from_le_bytes([data[off], data[off + 1]])
    }

    fn u32_at(data: &[u8], off: usize) -> u32 {
        u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
    }

    #[test]
    fn encodes_valid_ms_shllink_with_idlist_linkinfo_and_unicode_strings() {
        let out = Path::new("desktop.lnk");
        let bytes = encode_shortcut(&ShortcutSpec {
            out,
            target: r"C:\Apps\WuwaTappo\WuwaTappoLauncher.exe",
            arguments: Some("--minimized"),
            workdir: Some(r"C:\Apps\WuwaTappo"),
            icon: Some(r"C:\Apps\WuwaTappo\WuwaTappo.Local.exe"),
            description: Some("WuwaTappo 全图鉴版 1.3"),
            window: WindowMode::Minimized,
        });

        assert_eq!(u32_at(&bytes, 0), 0x4C);
        assert_eq!(&bytes[4..20], &LINK_CLSID);
        let flags = u32_at(&bytes, 20);
        assert_ne!(flags & FLAG_HAS_LINK_TARGET_ID_LIST, 0);
        assert_ne!(flags & FLAG_HAS_LINK_INFO, 0);
        assert_ne!(flags & FLAG_IS_UNICODE, 0);
        assert_eq!(u32_at(&bytes, 60), 7); // Minimized

        // Walk IDList
        let idlist_size = u16_at(&bytes, 76) as usize;
        let link_info_off = 78 + idlist_size;
        assert_eq!(u16_at(&bytes, link_info_off - 2), 0); // TerminalID

        // Walk LinkInfo
        let li_size = u32_at(&bytes, link_info_off) as usize;
        assert_eq!(u32_at(&bytes, link_info_off + 4), 36);

        // Walk StringData in exact MS-SHLLINK order
        let mut cursor = link_info_off + li_size;
        let mut read_str = || {
            let chars = u16_at(&bytes, cursor) as usize;
            cursor += 2;
            let units: Vec<u16> = (0..chars)
                .map(|i| u16_at(&bytes, cursor + i * 2))
                .collect();
            cursor += chars * 2;
            String::from_utf16(&units).unwrap()
        };
        assert_eq!(read_str(), "WuwaTappo 全图鉴版 1.3");
        assert_eq!(read_str(), r"C:\Apps\WuwaTappo");
        assert_eq!(read_str(), "--minimized");
        assert_eq!(read_str(), r"C:\Apps\WuwaTappo\WuwaTappo.Local.exe");
        assert_eq!(u32_at(&bytes, cursor), 0); // TerminalBlock
        assert_eq!(cursor + 4, bytes.len());
    }
}
