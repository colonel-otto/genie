use crate::{AOE_AOK, AOE1_DE, AOE2_DE, CPXVersion, CampaignHeader, ScenarioMeta};
use byteorder::{LE, WriteBytesExt};
use genie_scx::{DLCPackage, Result as SCXResult, Scenario};
use std::io::{self, Write};

/// Type for errors that could occur during writing.
#[derive(Debug, thiserror::Error)]
pub enum WriteCampaignError {
    /// An I/O error occurred during writing.
    #[error("{}", .0)]
    IoError(#[from] io::Error),
    /// A scenario could not be found, either because the original campaign file was corrupt, or
    /// the scenario file exists but could not be parsed.
    #[error("missing scenario data for index {}", .0)]
    NotFoundError(usize),
}

fn write_variable_str<W: Write>(output: &mut W, value: &str) -> io::Result<()> {
    output.write_u16::<LE>(0x0A60)?;
    let len = value.len();
    assert!(len < u16::MAX as usize);
    output.write_u16::<LE>(u16::try_from(len).expect("checked by the assert above"))?;
    output.write_all(value.as_bytes())?;
    Ok(())
}

/// Write the campaign header to the `output` stream.
fn write_campaign_header<W: Write>(header: &CampaignHeader, output: &mut W) -> io::Result<()> {
    assert!(header.num_scenarios < i32::MAX as usize);
    let num_scenarios = u32::try_from(header.num_scenarios).expect("checked by the assert above");

    output.write_all(&header.version)?;
    if header.version == AOE2_DE {
        // DE2 always supports all DLC, so we can just write them all out by default
        let dependencies = [
            DLCPackage::AgeOfKings,
            DLCPackage::AgeOfConquerors,
            DLCPackage::TheForgotten,
            DLCPackage::AfricanKingdoms,
            DLCPackage::RiseOfTheRajas,
            DLCPackage::LastKhans,
        ];
        output.write_u32::<LE>(u32::try_from(dependencies.len()).expect("six packages"))?;
        for dep in &dependencies {
            output.write_i32::<LE>(i32::from(*dep))?;
        }
        let mut name_bytes = header.name.as_bytes().to_vec();
        assert!(name_bytes.len() < 256);
        name_bytes.resize(256, 0);
        output.write_all(&name_bytes)?;
        output.write_u32::<LE>(num_scenarios)?;
    } else if header.version == AOE1_DE {
        output.write_u32::<LE>(num_scenarios)?;
        write_variable_str(output, &header.name)?;
    } else {
        let mut name_bytes = header.name.as_bytes().to_vec();
        assert!(name_bytes.len() < 256);
        name_bytes.resize(256, 0);
        output.write_all(&name_bytes)?;
        output.write_u32::<LE>(num_scenarios)?;
    }
    Ok(())
}

/// Write metadata for a single scenario into the `output` stream in the classic AoE/AoK format.
fn write_scenario_meta<W: Write>(meta: &ScenarioMeta, output: &mut W) -> io::Result<()> {
    assert!(meta.size < i32::MAX as usize);
    assert!(meta.offset < i32::MAX as usize);

    output.write_i32::<LE>(i32::try_from(meta.size).expect("checked by the assert above"))?;
    output.write_i32::<LE>(i32::try_from(meta.offset).expect("checked by the assert above"))?;

    let mut name_bytes = meta.name.as_bytes().to_vec();
    assert!(name_bytes.len() < 255);
    name_bytes.extend(vec![0; 255 - name_bytes.len()]);
    output.write_all(&name_bytes)?;

    let mut filename_bytes = meta.filename.as_bytes().to_vec();
    assert!(filename_bytes.len() < 255);
    filename_bytes.extend(vec![0; 255 - filename_bytes.len()]);
    output.write_all(&filename_bytes)?;

    Ok(())
}

/// Write metadata for a single scenario into the `output` stream, in the `AoE1`: DE format.
fn write_scenario_meta_de<W: Write>(meta: &ScenarioMeta, output: &mut W) -> io::Result<()> {
    // `u64::MAX as usize` is `usize::MAX` on every target.
    assert!(meta.size < usize::MAX);
    assert!(meta.offset < usize::MAX);

    output.write_u64::<LE>(meta.size as u64)?;
    output.write_u64::<LE>(meta.offset as u64)?;
    write_variable_str(output, &meta.name)?;
    write_variable_str(output, &meta.filename)?;
    Ok(())
}

/// Write metadata for a single scenario into the `output` stream, in the `AoE2`: DE format.
fn write_scenario_meta_de2<W: Write>(meta: &ScenarioMeta, output: &mut W) -> io::Result<()> {
    assert!(meta.size < u32::MAX as usize);
    assert!(meta.offset < u32::MAX as usize);

    output.write_u32::<LE>(u32::try_from(meta.size).expect("checked by the assert above"))?;
    output.write_u32::<LE>(u32::try_from(meta.offset).expect("checked by the assert above"))?;
    write_variable_str(output, &meta.name)?;
    write_variable_str(output, &meta.filename)?;
    Ok(())
}

/// Describes a scenario file to be added to the campaign.
#[derive(Debug)]
struct CampaignEntry {
    name: String,
    filename: String,
    bytes: Vec<u8>,
}

impl CampaignEntry {
    /// Get the user-visible name of this entry.
    fn name(&self) -> &str {
        &self.name
    }

    /// Get the file name of this entry.
    fn filename(&self) -> &str {
        &self.filename
    }

    /// Get the size in bytes of this entry.
    const fn size(&self) -> usize {
        self.bytes.len()
    }

    /// Get the byte array for this entry.
    fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// A campaign file writer. Instantiate it, then add scenario files to it.
///
/// This has to keep all scenario files in memory until the file is written on a call to
/// `flush()`.
#[derive(Debug)]
pub struct CampaignWriter<W: Write> {
    writer: W,
    header: CampaignHeader,
    scenarios: Vec<CampaignEntry>,
}

impl<W: Write> CampaignWriter<W> {
    /// Create a new campaign with user-visible name `name`, writing to the `writer` stream.
    pub fn new(name: &str, writer: W) -> Self {
        Self {
            writer,
            header: CampaignHeader::new(name),
            scenarios: vec![],
        }
    }

    /// Set the file version to output.
    #[must_use]
    pub fn version(mut self, version: CPXVersion) -> Self {
        debug_assert!(
            [AOE_AOK, AOE1_DE, AOE2_DE].contains(&version),
            "unknown or unsupported version"
        );
        self.header.version = version;
        self
    }

    /// Add a scenario (as a byte array) to this campaign.
    pub fn add_raw(&mut self, name: &str, filename: &str, scx: Vec<u8>) {
        self.scenarios.push(CampaignEntry {
            name: name.to_owned(),
            filename: filename.to_owned(),
            bytes: scx,
        });
    }

    /// Add a Scenario instance from genie-scx to this campaign.
    ///
    /// This returns a Result because it serializes the scenario to an in-memory byte array, which
    /// may fail.
    ///
    /// # Errors
    ///
    /// Returns an error if the scenario cannot be serialized.
    pub fn add(&mut self, name: &str, scx: &Scenario) -> SCXResult<()> {
        let mut bytes = vec![];
        scx.write_to(&mut bytes)?;
        self.scenarios.push(CampaignEntry {
            name: name.to_owned(),
            filename: scx.filename().to_owned(),
            bytes,
        });
        Ok(())
    }

    /// Consume the `CampaignWriter` instance, returning the inner `Write` instance.
    pub fn into_inner(self) -> W {
        self.writer
    }

    /// Write the campaign header.
    fn write_header(&mut self) -> io::Result<()> {
        self.header.num_scenarios = self.scenarios.len();
        write_campaign_header(&self.header, &mut self.writer)
    }

    /// Get the size in bytes of all metadata.
    const fn get_meta_size(&self) -> usize {
        // version + number of scenarios + campaign name
        let header_size = size_of::<CPXVersion>()
            + size_of::<i32>() // number of scenarios
            + 256; // campaign name
        header_size + self.scenarios.len() * (2 * size_of::<i32>() + 255 + 255)
    }

    /// Get the size in bytes of all metadata for a `AoE1`: DE campaign file.
    fn get_meta_size_de(&self) -> usize {
        // Length of a single variable string is (4 + byte length)
        const fn strlen(s: &str) -> usize {
            s.len() + 4
        }
        let header_size = size_of::<CPXVersion>()
            + size_of::<u32>() // number of scenarios
            + strlen(&self.header.name);
        self.scenarios.iter().fold(header_size, |acc, scen| {
            acc + 2 * size_of::<u64>() + strlen(&scen.name) + strlen(&scen.filename)
        })
    }

    /// Get the size in bytes of all metadata for a `AoE2`: DE campaign file.
    fn get_meta_size_de2(&self) -> usize {
        // Length of a single variable string is (4 + byte length)
        const fn strlen(s: &str) -> usize {
            s.len() + 4
        }
        let header_size = size_of::<CPXVersion>()
            + 7 * size_of::<i32>() // DLC dependencies
            + size_of::<u32>() // number of scenarios
            + 256; // campaign name
        self.scenarios.iter().fold(header_size, |acc, scen| {
            acc + 2 * size_of::<u32>() + strlen(&scen.name) + strlen(&scen.filename)
        })
    }

    /// Write the scenario metadata block.
    fn write_metas(&mut self) -> io::Result<()> {
        let write_meta = if self.header.version == AOE2_DE {
            write_scenario_meta_de2
        } else if self.header.version == AOE1_DE {
            write_scenario_meta_de
        } else {
            write_scenario_meta
        };

        // TODO: perhaps write to a buffer first instead of having these brittle calculations that
        // may get out of sync
        let mut offset = if self.header.version == AOE2_DE {
            self.get_meta_size_de2()
        } else if self.header.version == AOE1_DE {
            self.get_meta_size_de()
        } else {
            self.get_meta_size()
        };

        for scen in &self.scenarios {
            let meta = ScenarioMeta {
                size: scen.size(),
                offset,
                name: scen.name().to_owned(),
                filename: scen.filename().to_owned(),
            };
            write_meta(&meta, &mut self.writer)?;
            offset += scen.size();
        }
        Ok(())
    }

    /// Write the scenario data.
    fn write_scenarios(&mut self) -> io::Result<()> {
        for scen in &self.scenarios {
            self.writer.write_all(scen.bytes())?;
        }
        Ok(())
    }

    /// Write the scenarios to the output stream, consuming the `CampaignWriter` object.
    ///
    /// Returns the inner `Write`.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the output stream fails.
    ///
    /// # Panics
    ///
    /// Panics if the campaign has `i32::MAX` scenarios or more, or if a name does not fit the
    /// version's fields: 255 bytes or more for the campaign name (classic and `AoE2`: DE) or a
    /// scenario's name or file name (classic), `u16::MAX` bytes or more for a variable-length
    /// string, or a scenario size or offset too large for the version's integer width.
    pub fn flush(mut self) -> io::Result<W> {
        self.write_header()?;
        self.write_metas()?;
        self.write_scenarios()?;

        Ok(self.into_inner())
    }
}
