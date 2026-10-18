use super::{DRSHeader, DRSResource, DRSTable, DRSTableIterator, ResourceType};
use std::io::{Error, ErrorKind, Read, Seek, SeekFrom};

/// A DRS archive reader.
#[derive(Debug)]
pub struct DRSReader {
    header: Option<DRSHeader>,
    tables: Vec<DRSTable>,
}

impl DRSReader {
    /// Create a new DRS archive reader for the given handle.
    /// The handle must be `Read`able and `Seek`able.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the header, the table list or the resource directory fails.
    pub fn new<R>(handle: &mut R) -> Result<Self, Error>
    where
        R: Read + Seek,
    {
        let mut drs = Self {
            header: None,
            tables: vec![],
        };
        drs.read_header(handle)?;
        drs.read_tables(handle)?;
        drs.read_dictionary(handle)?;
        Ok(drs)
    }

    /// Read the DRS archive header.
    fn read_header<R: Read + Seek>(&mut self, handle: &mut R) -> Result<(), Error> {
        self.header = Some(DRSHeader::from(handle)?);
        Ok(())
    }

    /// Read the list of tables.
    fn read_tables<R: Read + Seek>(&mut self, handle: &mut R) -> Result<(), Error> {
        match self.header {
            Some(ref header) => {
                for _ in 0..header.num_resource_types {
                    let table = DRSTable::from(handle)?;
                    self.tables.push(table);
                }
            }
            None => panic!("must read header first"),
        }
        Ok(())
    }

    /// Read the list of resources.
    fn read_dictionary<R: Read + Seek>(&mut self, handle: &mut R) -> Result<(), Error> {
        for table in &mut self.tables {
            table.read_resources(handle)?;
        }
        Ok(())
    }

    /// Get the table for the given resource type.
    #[must_use]
    pub fn get_table(&self, resource_type: ResourceType) -> Option<&DRSTable> {
        self.tables
            .iter()
            .find(|table| table.resource_type == resource_type)
    }

    /// Get a resource of a given type and ID.
    #[must_use]
    pub fn get_resource(&self, resource_type: ResourceType, id: u32) -> Option<&DRSResource> {
        self.get_table(resource_type)
            .and_then(|table| table.get_resource(id))
    }

    /// Get the type of a resource with the given ID.
    #[must_use]
    pub fn get_resource_type(&self, id: u32) -> Option<ResourceType> {
        self.tables
            .iter()
            .find(|table| table.get_resource(id).is_some())
            .map(|table| table.resource_type)
    }

    /// Get a `Read`er for the given resource.
    ///
    /// It shares the file handle that is given, so make sure to use the return value before
    /// calling this method again.
    ///
    /// # Errors
    ///
    /// Returns an `ErrorKind::NotFound` error if the archive has no such resource, and an error
    /// if seeking to it fails.
    pub fn get_resource_reader<R: Read + Seek>(
        &self,
        mut handle: R,
        resource_type: ResourceType,
        id: u32,
    ) -> Result<impl Read, Error> {
        let &DRSResource { size, offset, .. } = self
            .get_resource(resource_type, id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "Resource not found in this archive"))?;

        let _ = handle.seek(SeekFrom::Start(u64::from(offset)))?;

        Ok(handle.take(u64::from(size)))
    }

    /// Read a file from the DRS archive.
    ///
    /// # Errors
    ///
    /// Returns an `ErrorKind::NotFound` error if the archive has no such resource, and an error
    /// if seeking to or reading it fails.
    pub fn read_resource<R: Read + Seek>(
        &self,
        handle: &mut R,
        resource_type: ResourceType,
        id: u32,
    ) -> Result<Box<[u8]>, Error> {
        let mut buf = vec![];

        // The byte count is `buf.len()`.
        let _ = self
            .get_resource_reader(handle, resource_type, id)?
            .read_to_end(&mut buf)?;

        Ok(buf.into_boxed_slice())
    }

    /// Iterate over the tables in this DRS archive.
    #[inline]
    pub fn tables(&self) -> DRSTableIterator<'_> {
        self.tables.iter()
    }
}
