//! The AI configuration (pattern: `AIConfig`, `UnknownAI`).

use binrw::{BinRead, binread};

/// The AI configuration; empty unless an AI script is loaded.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct AiConfig {
    pub has_ai: u32,
    #[br(if(has_ai == 1))]
    pub ai: Option<AiData>,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct AiData {
    pub max_strings: u16,
    #[br(temp)]
    strings_count: u16,
    pub unknown1: u32,
    #[br(count = strings_count)]
    pub strings: Vec<SizedString32>,
    pub unknown3: [u8; 3],
    pub unknown4: u16,
    pub unknown5: u16,
    pub unknown6: [u32; 2],
    pub rule_groups: [AiRuleGroup; 7],
    pub unknown21: [u32; 18],
    pub unknown22: [f32; 2],
    pub unknown23: [u32; 9],
    /// The pattern expects 656 words of `ff ff ff ff`, then 1024 zero words.
    #[br(count = 656 * 4, assert(padding_ff.iter().all(|b| *b == 0xff), "AI padding is not 0xff"))]
    pub padding_ff: Vec<u8>,
    #[br(count = 1024 * 4, assert(padding_zero.iter().all(|b| *b == 0), "AI padding is not zero"))]
    pub padding_zero: Vec<u8>,
}

/// A string with a u32 length.
#[binread]
#[derive(Debug, Clone, PartialEq, Eq)]
#[br(little)]
pub struct SizedString32 {
    #[br(temp)]
    len: u32,
    #[br(count = len)]
    pub bytes: Vec<u8>,
}

/// One of the seven groups of AI rules.
#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct AiRuleGroup {
    pub max: u16,
    #[br(temp)]
    count: u16,
    pub unknown: [u32; 3],
    #[br(parse_with = rules, args(count as usize))]
    pub rules: Vec<AiRule>,
}

#[binread]
#[derive(Debug, Clone, PartialEq)]
#[br(little)]
pub struct AiRule {
    pub seq: u16,
    #[br(magic = b"\xff\xff")]
    pub unknown1: u8,
    #[br(temp)]
    unknown_count: u8,
    #[br(magic = b"\x00\x00", count = unknown_count as usize * 6)]
    pub unknown2: Vec<u32>,
    pub unknown3: u32,
    pub unknown4: u32,
    pub unknown5: u32,
    pub crc: i32,
}

/// Up to `count` rules; a rule with crc -1 ends the group early (pattern: `break`).
#[binrw::parser(reader, endian)]
fn rules(count: usize) -> binrw::BinResult<Vec<AiRule>> {
    let mut out = Vec::new();
    while out.len() < count {
        let rule = AiRule::read_options(reader, endian, ())?;
        let last = rule.crc == -1;
        out.push(rule);
        if last {
            break;
        }
    }
    Ok(out)
}
