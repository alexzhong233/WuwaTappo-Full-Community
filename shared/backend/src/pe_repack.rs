//! Windows PE32+ static resource extractor and `.local`/`.lboot` repacker.
//!
//! Replaces `windows/src/tools/extract_static.py` and `windows/src/tools/pe_packer.py`
//! with pure Rust (zero Python / `pefile` / `Brotli` dependencies).

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{Cursor, Read},
    path::Path,
};

use crate::injector::PatchLogEntry;

const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
const IMAGE_NT_OPTIONAL_HDR64_MAGIC: u16 = 0x020B;
const IMAGE_DIRECTORY_ENTRY_IMPORT: usize = 1;
const IMAGE_DIRECTORY_ENTRY_RESOURCE: usize = 2;
const IMAGE_DIRECTORY_ENTRY_EXCEPTION: usize = 3;
const IMAGE_DIRECTORY_ENTRY_SECURITY: usize = 4;
const IMAGE_DIRECTORY_ENTRY_BASERELOC: usize = 5;
const IMAGE_REL_BASED_DIR64: u16 = 10;

fn r16(data: &[u8], off: usize) -> Result<u16, String> {
    let slice = data
        .get(off..off + 2)
        .ok_or_else(|| format!("PE 读取 u16 越界 @ {off:#x}"))?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn r32(data: &[u8], off: usize) -> Result<u32, String> {
    let slice = data
        .get(off..off + 4)
        .ok_or_else(|| format!("PE 读取 u32 越界 @ {off:#x}"))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn r64(data: &[u8], off: usize) -> Result<u64, String> {
    let slice = data
        .get(off..off + 8)
        .ok_or_else(|| format!("PE 读取 u64 越界 @ {off:#x}"))?;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn w16(data: &mut [u8], off: usize, val: u16) -> Result<(), String> {
    let slice = data
        .get_mut(off..off + 2)
        .ok_or_else(|| format!("PE 写入 u16 越界 @ {off:#x}"))?;
    slice.copy_from_slice(&val.to_le_bytes());
    Ok(())
}

fn w32(data: &mut [u8], off: usize, val: u32) -> Result<(), String> {
    let slice = data
        .get_mut(off..off + 4)
        .ok_or_else(|| format!("PE 写入 u32 越界 @ {off:#x}"))?;
    slice.copy_from_slice(&val.to_le_bytes());
    Ok(())
}

fn w64(data: &mut [u8], off: usize, val: u64) -> Result<(), String> {
    let slice = data
        .get_mut(off..off + 8)
        .ok_or_else(|| format!("PE 写入 u64 越界 @ {off:#x}"))?;
    slice.copy_from_slice(&val.to_le_bytes());
    Ok(())
}

fn align_up(n: usize, a: usize) -> Result<usize, String> {
    if a == 0 {
        return Err("PE 对齐粒度不能为 0".into());
    }
    Ok(((n + a - 1) / a) * a)
}

#[derive(Clone, Debug)]
struct PeSection {
    header_offset: usize,
    name: String,
    virtual_size: u32,
    rva: u32,
    raw_size: u32,
    raw_offset: u32,
}

impl PeSection {
    fn entropy(&self, data: &[u8]) -> f64 {
        let start = self.raw_offset as usize;
        let len = (self.raw_size as usize).min(data.len().saturating_sub(start));
        if len == 0 {
            return 0.0;
        }
        let mut counts = [0u64; 256];
        for &b in &data[start..start + len] {
            counts[b as usize] += 1;
        }
        let total = len as f64;
        let mut h = 0.0;
        for count in counts {
            if count > 0 {
                let p = count as f64 / total;
                h -= p * p.log2();
            }
        }
        (h * 1000.0).round() / 1000.0
    }
}

#[derive(Clone, Copy, Debug)]
struct DataDir {
    rva: u32,
    size: u32,
}

#[derive(Debug)]
struct Pe64 {
    coff_offset: usize,
    opt_offset: usize,
    machine: u16,
    number_of_sections: u16,
    entry_rva: u32,
    image_base: u64,
    section_alignment: u32,
    file_alignment: u32,
    size_of_code: u32,
    size_of_initialized_data: u32,
    size_of_headers: u32,
    data_directories: Vec<DataDir>,
    sections: Vec<PeSection>,
}

impl Pe64 {
    fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 0x40 || &data[0..2] != b"MZ" {
            return Err("不是有效的 PE 可执行文件（缺少 MZ 头）".into());
        }
        let pe_off = r32(data, 0x3C)? as usize;
        if data.get(pe_off..pe_off + 4) != Some(b"PE\0\0") {
            return Err("缺少 PE\\0\\0 签名".into());
        }
        let coff_offset = pe_off + 4;
        let machine = r16(data, coff_offset)?;
        if machine != IMAGE_FILE_MACHINE_AMD64 {
            return Err(format!("仅支持 x86_64 PE32+ 文件（machine={machine:#x}）"));
        }
        let number_of_sections = r16(data, coff_offset + 2)?;
        let size_of_opt = r16(data, coff_offset + 16)? as usize;
        let opt_offset = coff_offset + 20;
        let magic = r16(data, opt_offset)?;
        if magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC {
            return Err(format!("仅支持 PE32+（magic={magic:#x}）"));
        }
        let size_of_code = r32(data, opt_offset + 4)?;
        let size_of_initialized_data = r32(data, opt_offset + 8)?;
        let entry_rva = r32(data, opt_offset + 16)?;
        let image_base = r64(data, opt_offset + 24)?;
        let section_alignment = r32(data, opt_offset + 32)?;
        let file_alignment = r32(data, opt_offset + 36)?;
        let size_of_headers = r32(data, opt_offset + 60)?;
        let num_dirs = (r32(data, opt_offset + 108)? as usize).min(16);
        let dir_base = opt_offset + 112;
        let mut data_directories = Vec::with_capacity(num_dirs);
        for i in 0..num_dirs {
            data_directories.push(DataDir {
                rva: r32(data, dir_base + i * 8)?,
                size: r32(data, dir_base + i * 8 + 4)?,
            });
        }
        let sec_table = opt_offset + size_of_opt;
        let mut sections = Vec::with_capacity(number_of_sections as usize);
        for i in 0..number_of_sections as usize {
            let hdr = sec_table + i * 40;
            if hdr + 40 > data.len() {
                return Err("PE 节头表截断".into());
            }
            let raw_name = &data[hdr..hdr + 8];
            let end_nil = raw_name.iter().position(|&b| b == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&raw_name[..end_nil]).into_owned();
            sections.push(PeSection {
                header_offset: hdr,
                name,
                virtual_size: r32(data, hdr + 8)?,
                rva: r32(data, hdr + 12)?,
                raw_size: r32(data, hdr + 16)?,
                raw_offset: r32(data, hdr + 20)?,
            });
        }
        Ok(Self {
            coff_offset,
            opt_offset,
            machine,
            number_of_sections,
            entry_rva,
            image_base,
            section_alignment,
            file_alignment,
            size_of_code,
            size_of_initialized_data,
            size_of_headers,
            data_directories,
            sections,
        })
    }

    fn dir(&self, idx: usize) -> Option<DataDir> {
        self.data_directories
            .get(idx)
            .copied()
            .filter(|d| d.rva != 0 && d.size != 0)
    }

    fn rva_to_offset(&self, rva: u32) -> Option<usize> {
        if rva < self.size_of_headers {
            return Some(rva as usize);
        }
        for s in &self.sections {
            let span = s.virtual_size.max(s.raw_size);
            if rva >= s.rva && rva < s.rva.saturating_add(span) {
                let delta = rva - s.rva;
                if delta < s.raw_size {
                    return Some(s.raw_offset as usize + delta as usize);
                }
            }
        }
        None
    }

    fn offset_to_rva(&self, offset: usize) -> Option<u32> {
        if offset < self.size_of_headers as usize {
            return Some(offset as u32);
        }
        for s in &self.sections {
            let start = s.raw_offset as usize;
            let end = start.saturating_add(s.raw_size as usize);
            if offset >= start && offset < end {
                return Some(s.rva + (offset - start) as u32);
            }
        }
        None
    }

    fn get_data<'a>(&self, data: &'a [u8], rva: u32, size: usize) -> Option<&'a [u8]> {
        let off = self.rva_to_offset(rva)?;
        data.get(off..off.checked_add(size)?)
    }

    fn overlay_offset(&self, data: &[u8]) -> Option<usize> {
        let end_raw = self.end_of_sections_raw();
        (data.len() > end_raw).then_some(end_raw)
    }

    fn end_of_sections_raw(&self) -> usize {
        self.sections
            .iter()
            .filter(|s| s.raw_size > 0)
            .map(|s| s.raw_offset as usize + s.raw_size as usize)
            .max()
            .unwrap_or(0)
    }

    fn read_cstr(&self, data: &[u8], rva: u32) -> Option<String> {
        let off = self.rva_to_offset(rva)?;
        let tail = data.get(off..)?;
        let len = tail.iter().position(|&b| b == 0).unwrap_or(tail.len().min(512));
        Some(String::from_utf8_lossy(&tail[..len]).into_owned())
    }

    /// Parse `IMAGE_DIRECTORY_ENTRY_IMPORT` returning DLL -> symbols and name -> IAT RVA map.
    fn parse_imports(
        &self,
        data: &[u8],
    ) -> Result<(BTreeMap<String, Vec<String>>, HashMap<String, u32>), String> {
        let mut by_dll: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut iat_rvas: HashMap<String, u32> = HashMap::new();
        let Some(dir) = self.dir(IMAGE_DIRECTORY_ENTRY_IMPORT) else {
            return Ok((by_dll, iat_rvas));
        };
        let mut desc_off = self
            .rva_to_offset(dir.rva)
            .ok_or("IMPORT 目录 RVA 无效")?;
        while desc_off + 20 <= data.len() {
            let orig_first_thunk = r32(data, desc_off)?;
            let name_rva = r32(data, desc_off + 12)?;
            let first_thunk = r32(data, desc_off + 16)?;
            if orig_first_thunk == 0 && name_rva == 0 && first_thunk == 0 {
                break;
            }
            let dll_name = self
                .read_cstr(data, name_rva)
                .unwrap_or_else(|| "<unknown>".into());
            let ilt_rva = if orig_first_thunk != 0 {
                orig_first_thunk
            } else {
                first_thunk
            };
            let mut list = Vec::new();
            if let Some(mut ilt_off) = self.rva_to_offset(ilt_rva) {
                let mut idx = 0u32;
                while ilt_off + 8 <= data.len() {
                    let entry = r64(data, ilt_off)?;
                    if entry == 0 {
                        break;
                    }
                    let slot_rva = first_thunk.saturating_add(idx * 8);
                    if (entry & 0x8000_0000_0000_0000) != 0 {
                        list.push(format!("ordinal:{}", entry & 0xFFFF));
                    } else if let Some(sym) = self.read_cstr(data, (entry as u32).saturating_add(2))
                    {
                        iat_rvas.insert(sym.clone(), slot_rva);
                        list.push(sym);
                    }
                    ilt_off += 8;
                    idx += 1;
                }
            }
            by_dll.insert(dll_name, list);
            desc_off += 20;
        }
        Ok((by_dll, iat_rvas))
    }

    fn base_reloc_rvas(&self, data: &[u8]) -> Result<HashSet<u32>, String> {
        let mut rvas = HashSet::new();
        let Some(dir) = self.dir(IMAGE_DIRECTORY_ENTRY_BASERELOC) else {
            return Ok(rvas);
        };
        let base_off = self
            .rva_to_offset(dir.rva)
            .ok_or("BASERELOC 目录 RVA 无效")?;
        let mut pos = 0usize;
        let max_len = dir.size as usize;
        while pos + 8 <= max_len && base_off + pos + 8 <= data.len() {
            let page_rva = r32(data, base_off + pos)?;
            let block_size = r32(data, base_off + pos + 4)? as usize;
            if block_size < 8 || pos + block_size > max_len || base_off + pos + block_size > data.len()
            {
                break;
            }
            let count = (block_size - 8) / 2;
            for i in 0..count {
                let entry = r16(data, base_off + pos + 8 + i * 2)?;
                let kind = entry >> 12;
                let off = entry & 0x0FFF;
                if kind == IMAGE_REL_BASED_DIR64 {
                    rvas.insert(page_rva + off as u32);
                }
            }
            pos += block_size;
        }
        Ok(rvas)
    }
}

fn resource_type_name(id: u32) -> String {
    match id {
        1 => "RT_CURSOR",
        2 => "RT_BITMAP",
        3 => "RT_ICON",
        4 => "RT_MENU",
        5 => "RT_DIALOG",
        6 => "RT_STRING",
        7 => "RT_FONTDIR",
        8 => "RT_FONT",
        9 => "RT_ACCELERATOR",
        10 => "RT_RCDATA",
        11 => "RT_MESSAGETABLE",
        12 => "RT_GROUP_CURSOR",
        14 => "RT_GROUP_ICON",
        16 => "RT_VERSION",
        17 => "RT_DLGINCLUDE",
        19 => "RT_PLUGPLAY",
        20 => "RT_VXD",
        21 => "RT_ANICURSOR",
        22 => "RT_ANIICON",
        23 => "RT_HTML",
        24 => "RT_MANIFEST",
        _ => return id.to_string(),
    }
    .into()
}

#[derive(Clone)]
enum ResId {
    Id(u32),
    Name(String),
}

impl ResId {
    fn label(&self) -> String {
        match self {
            Self::Id(v) => v.to_string(),
            Self::Name(s) => s.clone(),
        }
    }
}

fn read_res_entries(
    data: &[u8],
    res_base: usize,
    dir_off: usize,
) -> Result<Vec<(ResId, bool, usize)>, String> {
    if dir_off + 16 > data.len() {
        return Err("RESOURCE 目录头越界".into());
    }
    let named = r16(data, dir_off + 12)? as usize;
    let ids = r16(data, dir_off + 14)? as usize;
    let total = named + ids;
    let mut entries = Vec::with_capacity(total);
    for i in 0..total {
        let entry_off = dir_off + 16 + i * 8;
        if entry_off + 8 > data.len() {
            break;
        }
        let name_or_id = r32(data, entry_off)?;
        let child = r32(data, entry_off + 4)?;
        let res_id = if (name_or_id & 0x8000_0000) != 0 {
            let str_off = res_base + (name_or_id & 0x7FFF_FFFF) as usize;
            let char_len = r16(data, str_off)? as usize;
            let mut u16s = Vec::with_capacity(char_len);
            for j in 0..char_len {
                u16s.push(r16(data, str_off + 2 + j * 2)?);
            }
            ResId::Name(String::from_utf16_lossy(&u16s))
        } else {
            ResId::Id(name_or_id)
        };
        let is_dir = (child & 0x8000_0000) != 0;
        let rel = res_base + (child & 0x7FFF_FFFF) as usize;
        entries.push((res_id, is_dir, rel));
    }
    Ok(entries)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn generate_pe_checksum(data: &[u8], checksum_offset: usize) -> u32 {
    let mut csum: u64 = 0;
    let mut idx = 0usize;
    while idx < data.len() {
        let b0 = if idx >= checksum_offset && idx < checksum_offset + 4 {
            0
        } else {
            data[idx]
        };
        let b1 = if idx + 1 >= data.len()
            || (idx + 1 >= checksum_offset && idx + 1 < checksum_offset + 4)
        {
            0
        } else {
            data[idx + 1]
        };
        csum += u16::from_le_bytes([b0, b1]) as u64;
        csum = (csum & 0xFFFF) + (csum >> 16);
        idx += 2;
    }
    csum = (csum & 0xFFFF) + (csum >> 16);
    csum = (csum & 0xFFFF) + (csum >> 16);
    (csum as u32).wrapping_add(data.len() as u32)
}

fn scan_printable_strings(data: &[u8], min_chars: usize) -> (Vec<(usize, String)>, Vec<(usize, String)>) {
    let mut utf8_hits = Vec::new();
    let mut idx = 0usize;
    while idx < data.len() {
        let remaining = &data[idx..];
        let mut end = 0usize;
        let mut count = 0usize;
        while end < remaining.len() {
            let b = remaining[end];
            if (0x20..=0x7E).contains(&b) || b == b'\t' {
                end += 1;
                count += 1;
            } else if b >= 0xC2 {
                let s_res = std::str::from_utf8(&remaining[end..]);
                let valid_ch = match s_res {
                    Ok(s) => s.chars().next(),
                    Err(e) => {
                        let valid = e.valid_up_to();
                        if valid > 0 {
                            std::str::from_utf8(&remaining[end..end + valid])
                                .ok()
                                .and_then(|s| s.chars().next())
                        } else {
                            None
                        }
                    }
                };
                if let Some(ch) = valid_ch.filter(|c| !c.is_control()) {
                    end += ch.len_utf8();
                    count += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        if count >= min_chars {
            let text = String::from_utf8_lossy(&remaining[..end]).into_owned();
            utf8_hits.push((idx, text));
            idx += end;
        } else {
            idx += end.max(1);
        }
    }

    let mut utf16_hits = Vec::new();
    let mut i = 0usize;
    while i + 1 < data.len() {
        if (0x20..=0x7E).contains(&data[i]) && data[i + 1] == 0 {
            let start = i;
            let mut chars = Vec::new();
            while i + 1 < data.len() && (0x20..=0x7E).contains(&data[i]) && data[i + 1] == 0 {
                chars.push(data[i]);
                i += 2;
            }
            if chars.len() >= min_chars {
                utf16_hits.push((start, String::from_utf8_lossy(&chars).into_owned()));
            }
        } else {
            i += 1;
        }
    }
    (utf8_hits, utf16_hits)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AssetEntry {
    pub path: String,
    pub table_offset: usize,
    pub file_offset: usize,
    pub compressed_size: usize,
    pub size: usize,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
struct ExtractManifestAsset {
    key: String,
    #[serde(rename = "relativePath")]
    relative_path: String,
    #[serde(rename = "tableOffset")]
    table_offset: usize,
    #[serde(rename = "dataOffset")]
    data_offset: usize,
    #[serde(rename = "compressedSize")]
    compressed_size: usize,
    size: usize,
}

#[derive(Clone, Debug, Deserialize)]
struct ExtractManifest {
    assets: Vec<ExtractManifestAsset>,
}

/// Extract PE resources, `.ico` icons, import symbols, string inventories, and asset metadata.
pub fn extract_pe_metadata(
    exe_path: &Path,
    extract_root: &Path,
    manifest_json: &str,
) -> Result<Value, String> {
    let data = fs::read(exe_path).map_err(|e| format!("读取 PE 失败：{e}"))?;
    let manifest: ExtractManifest =
        serde_json::from_str(manifest_json).map_err(|e| format!("解析提取清单失败：{e}"))?;
    let pe = Pe64::parse(&data)?;

    let frontend = extract_root.join("frontend");
    let analysis = extract_root.join("analysis");
    fs::create_dir_all(&analysis).map_err(|e| e.to_string())?;

    let mut inventory = Vec::with_capacity(manifest.assets.len());
    let mut ext_counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &manifest.assets {
        let content = fs::read(frontend.join(&entry.relative_path))
            .map_err(|e| format!("读取已提取前端文件 {} 失败：{e}", entry.relative_path))?;
        let ext = Path::new(&entry.key)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        *ext_counts.entry(ext).or_insert(0) += 1;
        inventory.push(AssetEntry {
            path: entry.key.clone(),
            table_offset: entry.table_offset,
            file_offset: entry.data_offset,
            compressed_size: entry.compressed_size,
            size: entry.size,
            sha256: sha256_hex(&content),
        });
    }

    let mut resource_inventory = Vec::new();
    if let Some(res_dir) = pe.dir(IMAGE_DIRECTORY_ENTRY_RESOURCE) {
        let res_base = pe
            .rva_to_offset(res_dir.rva)
            .ok_or("RESOURCE 目录 RVA 无效")?;
        let mut icons_by_id: HashMap<u32, Vec<u8>> = HashMap::new();
        let mut group_blobs: Vec<(String, Vec<u8>)> = Vec::new();

        for (typ_id, typ_is_dir, typ_off) in read_res_entries(&data, res_base, res_base)? {
            if !typ_is_dir {
                continue;
            }
            let raw_typ = match &typ_id {
                ResId::Id(v) => Some(*v),
                ResId::Name(_) => None,
            };
            let type_name = match &typ_id {
                ResId::Name(s) => s.clone(),
                ResId::Id(v) => resource_type_name(*v),
            };
            for (ident_id, id_is_dir, id_off) in read_res_entries(&data, res_base, typ_off)? {
                if !id_is_dir {
                    continue;
                }
                let resource_label = ident_id.label();
                let safe_name: String = resource_label
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                let langs = read_res_entries(&data, res_base, id_off)?;
                for (lang_idx, (lang_id, lang_is_dir, data_entry_off)) in langs.iter().enumerate() {
                    if *lang_is_dir || *data_entry_off + 16 > data.len() {
                        continue;
                    }
                    let data_rva = r32(&data, *data_entry_off)?;
                    let size = r32(&data, *data_entry_off + 4)? as usize;
                    let raw = pe
                        .get_data(&data, data_rva, size)
                        .ok_or_else(|| format!("资源数据越界：{type_name}/{resource_label}"))?;
                    let ext = if raw_typ == Some(24) { ".manifest" } else { ".bin" };
                    let dest = extract_root
                        .join("pe_resources")
                        .join(&type_name)
                        .join(format!("{safe_name}_{}{ext}", lang_id.label()));
                    if let Some(parent) = dest.parent() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    fs::write(&dest, raw).map_err(|e| e.to_string())?;
                    resource_inventory.push(json!({
                        "type": type_name,
                        "id": resource_label,
                        "language": match lang_id { ResId::Id(v) => json!(v), ResId::Name(s) => json!(s) },
                        "size": raw.len(),
                        "rva": data_rva,
                    }));
                    if lang_idx == 0 {
                        if raw_typ == Some(3) {
                            if let ResId::Id(id) = ident_id {
                                icons_by_id.insert(id, raw.to_vec());
                            }
                        } else if raw_typ == Some(14) {
                            group_blobs.push((safe_name.clone(), raw.to_vec()));
                        }
                    }
                }
            }
        }

        for (group_name, raw) in group_blobs {
            if raw.len() < 6 {
                continue;
            }
            let count = r16(&raw, 4)? as usize;
            if raw.len() < 6 + count * 14 {
                continue;
            }
            let mut header = raw[..6].to_vec();
            let mut images = Vec::new();
            let mut cursor = (6 + count * 16) as u32;
            let mut complete = true;
            for i in 0..count {
                let entry = &raw[6 + i * 14..20 + i * 14];
                let icon_id = r16(entry, 12)? as u32;
                let Some(blob) = icons_by_id.get(&icon_id) else {
                    complete = false;
                    break;
                };
                header.extend_from_slice(&entry[..8]);
                header.extend_from_slice(&(blob.len() as u32).to_le_bytes());
                header.extend_from_slice(&cursor.to_le_bytes());
                images.extend_from_slice(blob);
                cursor += blob.len() as u32;
            }
            if complete {
                let mut ico = header;
                ico.extend_from_slice(&images);
                fs::create_dir_all(extract_root.join("pe_resources")).map_err(|e| e.to_string())?;
                fs::write(
                    extract_root
                        .join("pe_resources")
                        .join(format!("icon_{group_name}.ico")),
                    ico,
                )
                .map_err(|e| e.to_string())?;
            }
        }
    }

    let (imports, _) = pe.parse_imports(&data)?;
    let sections: Vec<Value> = pe
        .sections
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "rva": s.rva,
                "raw_offset": s.raw_offset,
                "raw_size": s.raw_size,
                "virtual_size": s.virtual_size,
                "entropy": s.entropy(&data),
            })
        })
        .collect();

    let (utf8_strings, utf16_strings) = scan_printable_strings(&data, 6);
    let mut utf8_out = String::new();
    for (off, text) in &utf8_strings {
        utf8_out.push_str(&format!("0x{off:08x}\t{text}\n"));
    }
    fs::write(analysis.join("strings_utf8.txt"), utf8_out.as_bytes()).map_err(|e| e.to_string())?;
    let utf16_out: Vec<String> = utf16_strings
        .iter()
        .map(|(off, text)| format!("0x{off:08x}\t{text}"))
        .collect();
    fs::write(
        analysis.join("strings_utf16.txt"),
        utf16_out.join("\n").as_bytes(),
    )
    .map_err(|e| e.to_string())?;

    let compressed_size: usize = inventory.iter().map(|x| x.compressed_size).sum();
    let uncompressed_size: usize = inventory.iter().map(|x| x.size).sum();
    let native_function_entries = pe
        .dir(IMAGE_DIRECTORY_ENTRY_EXCEPTION)
        .map(|d| (d.size as usize) / 12)
        .unwrap_or(0);

    let canonical_input = exe_path
        .canonicalize()
        .unwrap_or_else(|_| exe_path.to_path_buf());
    let metadata = json!({
        "input": canonical_input.to_string_lossy(),
        "input_size": data.len(),
        "sha256": sha256_hex(&data),
        "machine": format!("{:#x}", pe.machine),
        "image_base": format!("{:#x}", pe.image_base),
        "entry_rva": format!("{:#x}", pe.entry_rva),
        "asset_table_offset": manifest.assets.first().map(|a| a.table_offset),
        "asset_count": inventory.len(),
        "compressed_size": compressed_size,
        "uncompressed_size": uncompressed_size,
        "extensions": ext_counts,
        "errors": [],
        "sections": sections,
        "string_count": utf8_strings.len(),
        "native_function_entries": native_function_entries,
        "overlay_offset": pe.overlay_offset(&data),
    });

    for (fname, val) in [
        ("metadata.json", &metadata),
        ("assets.json", &json!(inventory)),
        ("imports.json", &json!(imports)),
        ("resources.json", &json!(resource_inventory)),
    ] {
        fs::write(
            analysis.join(fname),
            serde_json::to_string_pretty(val).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(metadata)
}

fn replace_bytes_exact(data: &mut [u8], old: &[u8], new: &[u8], expected: Option<usize>) -> Result<usize, String> {
    if old.len() != new.len() || old.is_empty() {
        return Err("PE 字节替换必须等长且非空".into());
    }
    let positions: Vec<usize> = data
        .windows(old.len())
        .enumerate()
        .filter_map(|(i, w)| (w == old).then_some(i))
        .collect();
    if let Some(exp) = expected {
        if positions.len() != exp {
            return Err(format!(
                "PE 字节锚点数量异常：实际 {}，期望 {exp}",
                positions.len()
            ));
        }
    }
    for pos in &positions {
        data[*pos..*pos + new.len()].copy_from_slice(new);
    }
    Ok(positions.len())
}

fn brotli_compress_max(plain: &[u8]) -> Result<Vec<u8>, String> {
    let mut params = brotli::enc::BrotliEncoderParams::default();
    params.quality = 9;
    params.lgwin = 22;
    let mut compressed = Vec::new();
    brotli::BrotliCompress(&mut Cursor::new(plain), &mut compressed, &params)
        .map_err(|e| format!("Brotli 压缩失败：{e}"))?;
    Ok(compressed)
}

fn brotli_decompress_vec(packed: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    brotli::Decompressor::new(Cursor::new(packed), 4096)
        .read_to_end(&mut out)
        .map_err(|e| format!("Brotli 解压校验失败：{e}"))?;
    Ok(out)
}

struct StubAsm {
    code: Vec<u8>,
    labels: HashMap<&'static str, usize>,
    fixups: Vec<(usize, FixupTarget)>,
}

enum FixupTarget {
    Label(&'static str),
    Rva(u32),
}

impl StubAsm {
    fn new() -> Self {
        Self {
            code: Vec::new(),
            labels: HashMap::new(),
            fixups: Vec::new(),
        }
    }

    fn emit(&mut self, bytes: &[u8]) {
        self.code.extend_from_slice(bytes);
    }

    fn label(&mut self, name: &'static str) {
        self.labels.insert(name, self.code.len());
    }

    fn rel_label(&mut self, prefix: &[u8], target: &'static str) {
        self.emit(prefix);
        self.fixups.push((self.code.len(), FixupTarget::Label(target)));
        self.emit(&[0; 4]);
    }

    fn rel_rva(&mut self, prefix: &[u8], target: u32) {
        self.emit(prefix);
        self.fixups.push((self.code.len(), FixupTarget::Rva(target)));
        self.emit(&[0; 4]);
    }

    fn push_utf16_label(&mut self, label: &'static str, text: &str) {
        self.label(label);
        for unit in text.encode_utf16().chain(Some(0)) {
            self.code.extend_from_slice(&unit.to_le_bytes());
        }
    }

    fn finalize(mut self, boot_rva: u32) -> Result<Vec<u8>, String> {
        for (pos, target) in self.fixups {
            let dest = match target {
                FixupTarget::Label(name) => {
                    boot_rva as i64
                        + *self
                            .labels
                            .get(name)
                            .ok_or_else(|| format!("未知汇编标签：{name}"))?
                            as i64
                }
                FixupTarget::Rva(rva) => rva as i64,
            };
            let rel = dest - (boot_rva as i64 + pos as i64 + 4);
            let rel_i32 = i32::try_from(rel).map_err(|_| "Stub RIP 相对偏移溢出 i32")?;
            self.code[pos..pos + 4].copy_from_slice(&rel_i32.to_le_bytes());
        }
        Ok(self.code)
    }
}

fn write_section_header(
    data: &mut [u8],
    off: usize,
    name: &[u8; 8],
    virtual_size: u32,
    rva: u32,
    raw_size: u32,
    raw_offset: u32,
    characteristics: u32,
) -> Result<(), String> {
    if off + 40 > data.len() {
        return Err("PE 节头空间不足".into());
    }
    data[off..off + 8].copy_from_slice(name);
    w32(data, off + 8, virtual_size)?;
    w32(data, off + 12, rva)?;
    w32(data, off + 16, raw_size)?;
    w32(data, off + 20, raw_offset)?;
    w32(data, off + 24, 0)?;
    w32(data, off + 28, 0)?;
    w16(data, off + 32, 0)?;
    w16(data, off + 34, 0)?;
    w32(data, off + 36, characteristics)?;
    Ok(())
}

/// Repack modified JS assets into `.local`, install `.lboot` runtime-fallback stub,
/// rewrite process identifiers / mutexes, verify DIR64 relocs + Brotli round-trip,
/// and emit `WuwaTappo.Local.v2.exe` + `build_report_v2.json`.
pub fn repack_pe(
    build_root: &Path,
    base_dir: &Path,
    frontend_dir: &Path,
    shared_name: &str,
    main_name: &str,
    panel_name: &str,
    extra_patched_assets: &[String],
    patch_log: &[PatchLogEntry],
) -> Result<Value, String> {
    let source_path = base_dir.join("WuwaTappo.official.exe");
    let orig_bytes =
        fs::read(&source_path).map_err(|e| format!("读取官方主程序失败：{e}"))?;
    let input_sha256 = sha256_hex(&orig_bytes);
    let mut data = orig_bytes;
    let pe = Pe64::parse(&data)?;

    let sec_dir_file_off = pe.opt_offset + 112 + IMAGE_DIRECTORY_ENTRY_SECURITY * 8;
    let cert_dir = pe.dir(IMAGE_DIRECTORY_ENTRY_SECURITY);
    w32(&mut data, sec_dir_file_off, 0)?;
    w32(&mut data, sec_dir_file_off + 4, 0)?;

    let end_raw = pe.end_of_sections_raw();
    if let Some(cert) = cert_dir {
        if cert.rva as usize != end_raw {
            return Err(format!(
                "Authenticode 证书偏移与节末尾不一致：{} != {end_raw}",
                cert.rva
            ));
        }
    }
    if data.len() > end_raw {
        data.truncate(end_raw);
    }

    let file_align = pe.file_alignment as usize;
    let sec_align = pe.section_alignment as usize;
    let raw_start = align_up(data.len(), file_align)?;
    let max_sec_end_rva = pe
        .sections
        .iter()
        .map(|s| s.rva as usize + (s.virtual_size.max(s.raw_size)) as usize)
        .max()
        .unwrap_or(0);
    let new_rva = align_up(max_sec_end_rva, sec_align)?;

    let inventory: Vec<AssetEntry> = serde_json::from_str(
        &fs::read_to_string(base_dir.join("analysis/assets.json"))
            .map_err(|e| format!("读取 analysis/assets.json 失败：{e}"))?,
    )
    .map_err(|e| format!("解析 assets.json 失败：{e}"))?;

    let mut targets: Vec<String> = vec![
        shared_name.to_owned(),
        main_name.to_owned(),
        panel_name.to_owned(),
    ];
    targets.extend(extra_patched_assets.iter().cloned());

    let mut payload = Vec::new();
    let mut asset_changes = Vec::new();
    for name in &targets {
        let key = format!("/assets/{name}");
        let row = inventory
            .iter()
            .find(|x| x.path == key)
            .ok_or_else(|| format!("assets.json 缺少条目：{key}"))?;
        let plain = fs::read(frontend_dir.join("assets").join(name))
            .map_err(|e| format!("读取补丁资源 {name} 失败：{e}"))?;
        let packed = brotli_compress_max(&plain)?;
        while payload.len() % 8 != 0 {
            payload.push(0);
        }
        let asset_rva = new_rva + payload.len();
        payload.extend_from_slice(&packed);
        w64(&mut data, row.table_offset + 16, pe.image_base + asset_rva as u64)?;
        w64(&mut data, row.table_offset + 24, packed.len() as u64)?;

        let orig_asset = fs::read(base_dir.join("frontend/assets").join(name))
            .map_err(|e| format!("读取原版资源 {name} 失败：{e}"))?;
        let old_hash = BASE64.encode(Sha256::digest(&orig_asset));
        let new_hash = BASE64.encode(Sha256::digest(&plain));
        replace_bytes_exact(&mut data, old_hash.as_bytes(), new_hash.as_bytes(), Some(1))?;

        asset_changes.push(json!({
            "path": row.path,
            "rva": format!("{asset_rva:#x}"),
            "compressed_size": packed.len(),
            "size": plain.len(),
            "sha256": sha256_hex(&plain),
        }));
    }

    let mut renamed = 0usize;
    for (old, new, count_as_primary) in [
        (
            b"com.kuro.wuwatappo.cn".as_slice(),
            b"com.kuro.wuwatappo.lc".as_slice(),
            true,
        ),
        (
            b"com.kuro.wuwatappo.global".as_slice(),
            b"com.kuro.wuwatappo.localx".as_slice(),
            true,
        ),
        (
            b"wuwatappo.global".as_slice(),
            b"wuwatappo.localx".as_slice(),
            false,
        ),
    ] {
        let hits = replace_bytes_exact(&mut data, old, new, None)?;
        if count_as_primary {
            renamed += hits;
        }
    }
    if renamed == 0 {
        return Err("未找到可用于进程隔离的应用标识符".into());
    }

    for suffix in [b"-sic", b"-siw", b"-sim"] {
        let mut old = vec![0xC0, 0x04];
        old.extend_from_slice(suffix);
        old.push(0);
        let mut new = vec![0xC0, 0x04, b'-', b'f'];
        new.extend_from_slice(&suffix[2..]);
        new.push(0);
        replace_bytes_exact(&mut data, &old, &new, Some(1))?;
    }

    let last_sec_hdr = pe
        .sections
        .last()
        .ok_or("PE 没有节表")?
        .header_offset;
    let new_sec_hdr = last_sec_hdr + 40;
    let first_sec_raw = pe
        .sections
        .iter()
        .filter(|s| s.raw_offset > 0)
        .map(|s| s.raw_offset as usize)
        .min()
        .unwrap_or(usize::MAX);
    if new_sec_hdr + 80 > first_sec_raw {
        return Err("PE 节表尾空隙不足 80 字节，无法追加 .local 与 .lboot".into());
    }
    if data[new_sec_hdr..new_sec_hdr + 80].iter().any(|&b| b != 0) {
        return Err("PE 节表尾空隙非零，拒绝覆盖".into());
    }

    let local_raw_size = align_up(payload.len(), file_align)?;
    write_section_header(
        &mut data,
        new_sec_hdr,
        b".local\0\0",
        payload.len() as u32,
        new_rva as u32,
        local_raw_size as u32,
        raw_start as u32,
        0x4000_0040,
    )?;
    data.resize(raw_start, 0);
    data.extend_from_slice(&payload);
    data.resize(raw_start + local_raw_size, 0);

    let boot_rva = align_up(new_rva + payload.len(), sec_align)? as u32;
    let boot_raw = align_up(data.len(), file_align)?;
    let (_, iat) = pe.parse_imports(&data)?;
    let get_mod = *iat
        .get("GetModuleFileNameW")
        .ok_or("导入表缺少 GetModuleFileNameW")?;
    let get_attr = *iat
        .get("GetFileAttributesW")
        .ok_or("导入表缺少 GetFileAttributesW")?;
    let set_env = *iat
        .get("SetEnvironmentVariableW")
        .ok_or("导入表缺少 SetEnvironmentVariableW")?;
    let exit_proc = *iat.get("ExitProcess").ok_or("导入表缺少 ExitProcess")?;

    let mut asm = StubAsm::new();
    asm.emit(&[
        0x9C, 0x50, 0x51, 0x52, 0x41, 0x50, 0x41, 0x51, 0x41, 0x52, 0x41, 0x53, 0x53, 0x56, 0x57,
        0x48, 0x81, 0xEC, 0x50, 0x04, 0x00, 0x00, 0xFC,
    ]);
    asm.emit(&[
        0x31, 0xC9, 0x48, 0x8D, 0x54, 0x24, 0x20, 0x41, 0xB8, 0x08, 0x02, 0x00, 0x00,
    ]);
    asm.rel_rva(&[0xFF, 0x15], get_mod);
    asm.emit(&[0x85, 0xC0]);
    asm.rel_label(&[0x0F, 0x84], "failure");
    asm.emit(&[0x3D, 0xF0, 0x01, 0x00, 0x00]);
    asm.rel_label(&[0x0F, 0x83], "failure");
    asm.emit(&[0x89, 0xC3, 0x48, 0x8D, 0x7C, 0x24, 0x20]);
    asm.label("find_separator");
    asm.emit(&[0xFF, 0xCB]);
    asm.rel_label(&[0x0F, 0x88], "failure");
    asm.emit(&[0x66, 0x83, 0x3C, 0x5F, 0x5C]);
    asm.rel_label(&[0x0F, 0x85], "find_separator");
    asm.emit(&[0xFF, 0xC3]);

    for (name_label, suffix_label, loop_label, is_runtime) in [
        ("runtime_name", "runtime_suffix", "runtime_name_copy", true),
        ("profile_name", "profile_suffix", "profile_name_copy", false),
    ] {
        asm.emit(&[0x48, 0x8D, 0x7C, 0x24, 0x20, 0x48, 0x8D, 0x3C, 0x5F]);
        asm.rel_label(&[0x48, 0x8D, 0x35], suffix_label);
        asm.label(loop_label);
        asm.emit(&[0x66, 0xAD, 0x66, 0xAB, 0x66, 0x85, 0xC0]);
        asm.rel_label(&[0x0F, 0x85], loop_label);
        if is_runtime {
            asm.emit(&[0x48, 0x8D, 0x4C, 0x24, 0x20]);
            asm.rel_rva(&[0xFF, 0x15], get_attr);
            asm.emit(&[0x3D, 0xFF, 0xFF, 0xFF, 0xFF]);
            asm.rel_label(&[0x0F, 0x84], "skip_runtime");
        }
        asm.rel_label(&[0x48, 0x8D, 0x0D], name_label);
        asm.emit(&[0x48, 0x8D, 0x54, 0x24, 0x20]);
        asm.rel_rva(&[0xFF, 0x15], set_env);
        asm.emit(&[0x85, 0xC0]);
        asm.rel_label(&[0x0F, 0x84], "failure");
        if is_runtime {
            asm.label("skip_runtime");
        }
    }

    asm.emit(&[
        0x48, 0x81, 0xC4, 0x50, 0x04, 0x00, 0x00, 0x5F, 0x5E, 0x5B, 0x41, 0x5B, 0x41, 0x5A, 0x41,
        0x59, 0x41, 0x58, 0x5A, 0x59, 0x58, 0x9D,
    ]);
    asm.rel_rva(&[0xE9], pe.entry_rva);
    asm.label("failure");
    asm.emit(&[0xB9, 0x01, 0x00, 0x00, 0x00]);
    asm.rel_rva(&[0xFF, 0x15], exit_proc);
    asm.emit(&[0x0F, 0x0B]);

    asm.push_utf16_label("runtime_name", "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER");
    asm.push_utf16_label("runtime_suffix", "runtime");
    asm.push_utf16_label("profile_name", "WEBVIEW2_USER_DATA_FOLDER");
    asm.push_utf16_label("profile_suffix", "profile");

    let code = asm.finalize(boot_rva)?;
    let boot_raw_size = align_up(code.len(), file_align)?;
    write_section_header(
        &mut data,
        new_sec_hdr + 40,
        b".lboot\0\0",
        code.len() as u32,
        boot_rva,
        boot_raw_size as u32,
        boot_raw as u32,
        0x6000_0020,
    )?;

    w16(&mut data, pe.coff_offset + 2, pe.number_of_sections + 2)?;
    w32(&mut data, pe.opt_offset + 16, boot_rva)?;
    w32(
        &mut data,
        pe.opt_offset + 4,
        pe.size_of_code.saturating_add(boot_raw_size as u32),
    )?;
    w32(
        &mut data,
        pe.opt_offset + 8,
        pe.size_of_initialized_data
            .saturating_add(local_raw_size as u32),
    )?;
    let size_of_image = align_up(boot_rva as usize + code.len(), sec_align)? as u32;
    w32(&mut data, pe.opt_offset + 56, size_of_image)?;

    data.resize(boot_raw, 0);
    data.extend_from_slice(&code);
    data.resize(boot_raw + boot_raw_size, 0);

    let checksum_off = pe.opt_offset + 64;
    let checksum = generate_pe_checksum(&data, checksum_off);
    w32(&mut data, checksum_off, checksum)?;

    let out_path = build_root.join("WuwaTappo.Local.v2.exe");
    fs::write(&out_path, &data).map_err(|e| format!("写入打包 EXE 失败：{e}"))?;

    // Self-verify ASLR DIR64 relocations & Brotli round-trip on the emitted binary
    let patched_pe = Pe64::parse(&data)?;
    let relocs = patched_pe.base_reloc_rvas(&data)?;
    for (idx, name) in targets.iter().enumerate() {
        let key = format!("/assets/{name}");
        let row = inventory.iter().find(|x| x.path == key).unwrap();
        let ptr_rva = patched_pe
            .offset_to_rva(row.table_offset + 16)
            .ok_or_else(|| format!("资产表偏移无法映射到 RVA：{key}"))?;
        if !relocs.contains(&ptr_rva) {
            return Err(format!("ASLR 校验失败：{key} 指针 RVA {ptr_rva:#x} 不在 DIR64 重定位表内"));
        }
        let va = r64(&data, row.table_offset + 16)?;
        let len = r64(&data, row.table_offset + 24)? as usize;
        let asset_rva = u32::try_from(va.saturating_sub(patched_pe.image_base))
            .map_err(|_| "资产 VA 计算异常")?;
        let slice = patched_pe
            .get_data(&data, asset_rva, len)
            .ok_or_else(|| format!("打包后资产切片越界：{key}"))?;
        let plain = brotli_decompress_vec(slice)?;
        let expected_sha = asset_changes[idx]["sha256"].as_str().unwrap();
        if sha256_hex(&plain) != expected_sha {
            return Err(format!("Brotli 往返哈希校验失败：{key}"));
        }
    }

    let official_sha_now = sha256_hex(
        &fs::read(&source_path).map_err(|e| format!("复核官方 EXE 失败：{e}"))?,
    );
    if official_sha_now != input_sha256 {
        return Err("输入官方 EXE 在打包期间被意外修改".into());
    }

    let report = json!({
        "input_sha256": input_sha256,
        "output_sha256": sha256_hex(&data),
        "output_size": data.len(),
        "assets": asset_changes,
        "patches": patch_log,
        "certificate_removed": true,
        "aslr_relocations_verified": true,
        "original_modified": false,
        "private_webview_entry_stub": {
            "rva": format!("{boot_rva:#x}"),
            "size": code.len(),
            "original_entry_rva": format!("{:#x}", pe.entry_rva),
            "runtime_fallback": true,
        }
    });
    fs::write(
        build_root.join("build_report_v2.json"),
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_synthetic_pe64() -> (Vec<u8>, Vec<u8>, u32, usize) {
        // Synthetic PE32+: headers (0x400), .text (RVA 0x1000, raw 0x400, size 0x200),
        // .rdata (RVA 0x2000, raw 0x600, size 0x400), .reloc (RVA 0x3000, raw 0xA00, size 0x200),
        // .rsrc (RVA 0x4000, raw 0xC00, size 0x200), trailing mock Authenticode cert (0x20 bytes).
        let mut pe = vec![0u8; 0xE20];
        pe[0..2].copy_from_slice(b"MZ");
        w32(&mut pe, 0x3C, 0x80).unwrap();
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        w16(&mut pe, coff, IMAGE_FILE_MACHINE_AMD64).unwrap();
        w16(&mut pe, coff + 2, 4).unwrap(); // 4 sections (leaves >= 80 zero bytes before raw 0x400!)
        w16(&mut pe, coff + 16, 240).unwrap(); // SizeOfOptionalHeader
        let opt = coff + 20;
        w16(&mut pe, opt, IMAGE_NT_OPTIONAL_HDR64_MAGIC).unwrap();
        w32(&mut pe, opt + 4, 0x200).unwrap(); // SizeOfCode
        w32(&mut pe, opt + 8, 0x800).unwrap(); // SizeOfInitializedData
        w32(&mut pe, opt + 16, 0x1000).unwrap(); // AddressOfEntryPoint
        w64(&mut pe, opt + 24, 0x1_4000_0000).unwrap(); // ImageBase
        w32(&mut pe, opt + 32, 0x1000).unwrap(); // SectionAlignment
        w32(&mut pe, opt + 36, 0x200).unwrap(); // FileAlignment
        w32(&mut pe, opt + 56, 0x5000).unwrap(); // SizeOfImage
        w32(&mut pe, opt + 60, 0x400).unwrap(); // SizeOfHeaders
        w32(&mut pe, opt + 108, 16).unwrap(); // NumberOfRvaAndSizes

        let dirs = opt + 112;
        // Import directory at RVA 0x2100
        w32(&mut pe, dirs + IMAGE_DIRECTORY_ENTRY_IMPORT * 8, 0x2100).unwrap();
        w32(&mut pe, dirs + IMAGE_DIRECTORY_ENTRY_IMPORT * 8 + 4, 40).unwrap();
        // Resource directory at RVA 0x4000
        w32(&mut pe, dirs + IMAGE_DIRECTORY_ENTRY_RESOURCE * 8, 0x4000).unwrap();
        w32(&mut pe, dirs + IMAGE_DIRECTORY_ENTRY_RESOURCE * 8 + 4, 0x100).unwrap();
        // Security cert at file offset 0xE00
        w32(&mut pe, dirs + IMAGE_DIRECTORY_ENTRY_SECURITY * 8, 0xE00).unwrap();
        w32(&mut pe, dirs + IMAGE_DIRECTORY_ENTRY_SECURITY * 8 + 4, 0x20).unwrap();
        // BaseReloc at RVA 0x3000
        w32(&mut pe, dirs + IMAGE_DIRECTORY_ENTRY_BASERELOC * 8, 0x3000).unwrap();
        w32(&mut pe, dirs + IMAGE_DIRECTORY_ENTRY_BASERELOC * 8 + 4, 12).unwrap();

        let sec0 = opt + 240;
        let defs = [
            (*b".text\0\0\0", 0x1000u32, 0x200u32, 0x400u32, 0x6000_0020u32),
            (*b".rdata\0\0", 0x2000, 0x400, 0x600, 0x4000_0040),
            (*b".reloc\0\0", 0x3000, 0x200, 0xA00, 0x4200_0040),
            (*b".rsrc\0\0\0", 0x4000, 0x200, 0xC00, 0x4000_0040),
        ];
        for (i, (name, rva, size, raw, ch)) in defs.iter().enumerate() {
            write_section_header(&mut pe, sec0 + i * 40, name, *size, *rva, *size, *raw, *ch)
                .unwrap();
        }
        assert!(sec0 + 4 * 40 + 80 <= 0x400);

        // .rdata @ 0x600: asset table entry (0x600, pointer at 0x610 = RVA 0x2010), hashes, id, mutexes
        let table_offset = 0x600usize;
        let orig_asset = b"console.log('orig');".to_vec();
        let orig_hash = BASE64.encode(Sha256::digest(&orig_asset));
        pe[0x630..0x630 + 44].copy_from_slice(orig_hash.as_bytes());
        pe[0x660..0x660 + 21].copy_from_slice(b"com.kuro.wuwatappo.cn");
        pe[0x680..0x687].copy_from_slice(b"\xc0\x04-sic\0");
        pe[0x690..0x697].copy_from_slice(b"\xc0\x04-siw\0");
        pe[0x6A0..0x6A7].copy_from_slice(b"\xc0\x04-sim\0");

        // Import descriptor @ RVA 0x2100 (offset 0x700): KERNEL32.dll -> 4 needed APIs
        w32(&mut pe, 0x700, 0x2130).unwrap(); // OriginalFirstThunk
        w32(&mut pe, 0x700 + 12, 0x2128).unwrap(); // Name RVA
        w32(&mut pe, 0x700 + 16, 0x2160).unwrap(); // FirstThunk (IAT)
        pe[0x728..0x728 + 12].copy_from_slice(b"KERNEL32.dll");
        let syms = [
            "GetModuleFileNameW",
            "GetFileAttributesW",
            "SetEnvironmentVariableW",
            "ExitProcess",
        ];
        let mut str_rva = 0x2190u32;
        for (i, sym) in syms.iter().enumerate() {
            w64(&mut pe, 0x730 + i * 8, str_rva as u64).unwrap();
            let off = 0x600 + (str_rva - 0x2000) as usize;
            pe[off + 2..off + 2 + sym.len()].copy_from_slice(sym.as_bytes());
            str_rva += (2 + sym.len() + 2) as u32;
        }

        // DIR64 reloc @ RVA 0x3000 (offset 0xA00) covering RVA 0x2010 (pointer_rva = table_offset + 16)
        w32(&mut pe, 0xA00, 0x2000).unwrap();
        w32(&mut pe, 0xA04, 12).unwrap();
        w16(&mut pe, 0xA08, (IMAGE_REL_BASED_DIR64 << 12) | 0x010).unwrap();

        // Resource tree @ RVA 0x4000 (offset 0xC00): RT_ICON(3)/id=1 and RT_GROUP_ICON(14)/id=32512
        // Root dir @ 0x00: 2 id entries -> @ 0x10 (type 3 -> dir 0x20), @ 0x18 (type 14 -> dir 0x48)
        w16(&mut pe, 0xC00 + 14, 2).unwrap();
        w32(&mut pe, 0xC10, 3).unwrap();
        w32(&mut pe, 0xC14, 0x8000_0020).unwrap();
        w32(&mut pe, 0xC18, 14).unwrap();
        w32(&mut pe, 0xC1C, 0x8000_0048).unwrap();

        // Type 3 id dir @ 0x20 -> entry id=1 -> lang dir 0x30 -> lang 1033 -> data entry 0x40 (offset 0xA0)
        w16(&mut pe, 0xC20 + 14, 1).unwrap();
        w32(&mut pe, 0xC30, 1).unwrap();
        w32(&mut pe, 0xC34, 0x8000_0038).unwrap();
        w16(&mut pe, 0xC38 + 14, 1).unwrap();
        w32(&mut pe, 0xC48, 1033).unwrap();
        w32(&mut pe, 0xC4C, 0x0000_0080).unwrap();

        // Type 14 id dir @ 0x48 -> id=32512 -> lang dir 0x60 -> lang 1033 -> data entry 0x90
        w16(&mut pe, 0xC48 + 14 + 0x10, 0).unwrap(); // keep clean at 0x50..
        w16(&mut pe, 0xC50 + 14, 1).unwrap(); // wait use 0x50 instead!
        // Rewrite clean offsets for type 14 -> 0x50:
        w32(&mut pe, 0xC1C, 0x8000_0050).unwrap();
        w16(&mut pe, 0xC50 + 14, 1).unwrap();
        w32(&mut pe, 0xC60, 32512).unwrap();
        w32(&mut pe, 0xC64, 0x8000_0068).unwrap();
        w16(&mut pe, 0xC68 + 14, 1).unwrap();
        w32(&mut pe, 0xC78, 1033).unwrap();
        w32(&mut pe, 0xC7C, 0x0000_0090).unwrap();

        // Leaf DATA_ENTRY @ 0x80 (icon 1 payload @ RVA 0x40A0, size 4 = "PNG!")
        w32(&mut pe, 0xC80, 0x40A0).unwrap();
        w32(&mut pe, 0xC84, 4).unwrap();
        pe[0xCA0..0xCA4].copy_from_slice(b"PNG!");

        // Leaf DATA_ENTRY @ 0x90 (group icon 32512 @ RVA 0x40B0, size 20 = 6+14 bytes pointing to icon 1)
        w32(&mut pe, 0xC90, 0x40B0).unwrap();
        w32(&mut pe, 0xC94, 20).unwrap();
        w16(&mut pe, 0xCB2, 1).unwrap(); // type=1 ico
        w16(&mut pe, 0xCB4, 1).unwrap(); // count=1
        pe[0xCB6] = 16; // width
        pe[0xCB7] = 16; // height
        w16(&mut pe, 0xCBC + 6, 1).unwrap(); // icon_id = 1 at entry +12 (0xCB6 + 12 = 0xCC2)

        (pe, orig_asset, 0x2010, table_offset)
    }

    #[test]
    fn extracts_and_repacks_synthetic_pe64() {
        let dir = std::env::temp_dir().join(format!(
            "wuwa-pe-repack-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        let base = dir.join("base");
        let build = dir.join("build");
        let orig_assets = base.join("frontend/assets");
        let patched_assets = build.join("frontend_v2/assets");
        fs::create_dir_all(&orig_assets).unwrap();
        fs::create_dir_all(&patched_assets).unwrap();

        let (pe_bytes, orig_asset, _ptr_rva, table_offset) = build_synthetic_pe64();
        let exe_path = base.join("WuwaTappo.official.exe");
        fs::write(&exe_path, &pe_bytes).unwrap();
        fs::write(orig_assets.join("index-shared.js"), &orig_asset).unwrap();
        fs::write(orig_assets.join("main.js"), b"const m=1;").unwrap();
        fs::write(orig_assets.join("panel.js"), b"const p=1;").unwrap();

        let manifest = json!({
            "assets": [{
                "key": "/assets/index-shared.js",
                "relativePath": "assets/index-shared.js",
                "tableOffset": table_offset,
                "dataOffset": 0,
                "compressedSize": orig_asset.len(),
                "size": orig_asset.len(),
            }]
        })
        .to_string();
        let meta = extract_pe_metadata(&exe_path, &base, &manifest).unwrap();
        assert_eq!(meta["asset_count"], 1);
        assert!(base.join("pe_resources/icon_32512.ico").exists());

        // Only index-shared.js needs to be in assets.json if we also register main/panel; let's test 1 shared asset
        fs::write(
            patched_assets.join("index-shared.js"),
            b"console.log('patched community!');",
        )
        .unwrap();
        // To test repack_pe with only shared + existing entries, supply empty main/panel or register them
        // Note repack_pe expects shared/main/panel all in assets.json: use a 3-entry test or pass same name
        let report = repack_pe(
            &build,
            &base,
            &build.join("frontend_v2"),
            "index-shared.js",
            "index-shared.js",
            "index-shared.js",
            &[],
            &[],
        );
        // 2nd iteration of same shared name would try replacing already-replaced old_hash! Good check:
        assert!(report.is_err());

        // Now test normal single-asset + 2 dummy reloc assets? Even simpler: verify with distinct hashes
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn repack_pe_succeeds_end_to_end_and_preserves_original_binary() {
        let dir = std::env::temp_dir().join(format!(
            "wuwa-pe-repack-ok-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        let base = dir.join("base");
        let build = dir.join("build");
        let orig_assets = base.join("frontend/assets");
        let patched_assets = build.join("frontend_v2/assets");
        fs::create_dir_all(&orig_assets).unwrap();
        fs::create_dir_all(&patched_assets).unwrap();

        let (mut pe_bytes, s_orig, _ptr_rva, table_offset) = build_synthetic_pe64();
        let m_orig = b"console.log('main');";
        let p_orig = b"console.log('panel');";
        let m_hash = BASE64.encode(Sha256::digest(m_orig));
        let p_hash = BASE64.encode(Sha256::digest(p_orig));
        pe_bytes[0x8B0..0x8B0 + 44].copy_from_slice(m_hash.as_bytes());
        pe_bytes[0x8F0..0x8F0 + 44].copy_from_slice(p_hash.as_bytes());
        // Place m.js table at 0x810 (ptr at 0x820 -> RVA 0x2220) and p.js table at 0x830 (ptr at 0x840 -> RVA 0x2240)
        // cleanly after import strings (0x790..0x805) and before hashes (0x8B0..0x920).
        let dirs_basereloc = 0x84 + 20 + 112 + IMAGE_DIRECTORY_ENTRY_BASERELOC * 8;
        w32(&mut pe_bytes, dirs_basereloc + 4, 16).unwrap();
        w32(&mut pe_bytes, 0xA04, 16).unwrap();
        w16(&mut pe_bytes, 0xA0A, (IMAGE_REL_BASED_DIR64 << 12) | 0x220).unwrap();
        w16(&mut pe_bytes, 0xA0C, (IMAGE_REL_BASED_DIR64 << 12) | 0x240).unwrap();

        let exe_path = base.join("WuwaTappo.official.exe");
        fs::write(&exe_path, &pe_bytes).unwrap();
        fs::write(orig_assets.join("s.js"), &s_orig).unwrap();
        fs::write(orig_assets.join("m.js"), m_orig).unwrap();
        fs::write(orig_assets.join("p.js"), p_orig).unwrap();

        let manifest = json!({
            "assets": [
                {"key": "/assets/s.js", "relativePath": "assets/s.js", "tableOffset": table_offset, "dataOffset": 0, "compressedSize": s_orig.len(), "size": s_orig.len()},
                {"key": "/assets/m.js", "relativePath": "assets/m.js", "tableOffset": 0x810, "dataOffset": 0, "compressedSize": m_orig.len(), "size": m_orig.len()},
                {"key": "/assets/p.js", "relativePath": "assets/p.js", "tableOffset": 0x830, "dataOffset": 0, "compressedSize": p_orig.len(), "size": p_orig.len()}
            ]
        })
        .to_string();
        extract_pe_metadata(&exe_path, &base, &manifest).unwrap();

        fs::write(patched_assets.join("s.js"), b"console.log('s_patched');").unwrap();
        fs::write(patched_assets.join("m.js"), b"console.log('m_patched');").unwrap();
        fs::write(patched_assets.join("p.js"), b"console.log('p_patched');").unwrap();

        let report = repack_pe(
            &build,
            &base,
            &build.join("frontend_v2"),
            "s.js",
            "m.js",
            "p.js",
            &[],
            &[PatchLogEntry {
                anchor: "test".into(),
                matches: 1,
            }],
        )
        .unwrap();

        assert_eq!(report["certificate_removed"], true);
        assert_eq!(report["aslr_relocations_verified"], true);
        assert_eq!(report["original_modified"], false);
        assert_eq!(report["private_webview_entry_stub"]["runtime_fallback"], true);
        let out_pe = fs::read(build.join("WuwaTappo.Local.v2.exe")).unwrap();
        assert!(out_pe.windows(21).any(|w| w == b"com.kuro.wuwatappo.lc"));
        assert!(!out_pe.windows(21).any(|w| w == b"com.kuro.wuwatappo.cn"));
        assert!(out_pe.windows(7).any(|w| w == b"\xc0\x04-fic\0"));
        let _ = fs::remove_dir_all(&dir);
    }
}
