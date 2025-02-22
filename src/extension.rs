//! This module contains definitions for the extension and related types.
//! Extensions are optional data frames sitting before the voxel data.
//! When present, an extender frame of 4 bytes is also present at the
//! end of the NIFTI-1 header, with the first byte set to something
//! other than 0.

use crate::error::{NiftiError, Result};
use byteordered::{ByteOrdered, Endian};
use num_derive::FromPrimitive;
use std::io::{ErrorKind as IoErrorKind, Read};

/// Data type for representing a NIfTI-1.1 extension code
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, FromPrimitive)]
#[repr(u32)]
pub enum NiftiEcode {
    /// Ignore the extension
    NiftEcodeIgnore = 0,
    /// DICOM
    NiftiEcodeDicom = 2,
    /// AFNI extension in XML format
    NiftiEcodeAFNI = 4,
    /// String Comment
    NiftiEcodeComment = 6,
    /// XCEDE extension in XML format
    NiftiEcodeXCEDE = 8,
    /// JimDimInfo
    NiftiEcodeJimDimInfo = 10,
    /// WorkflowFWDS
    NiftiEcodeWorkflowFWDS = 12,
    /// Freesurfer
    NiftiEcodeFreesurfer = 14,
}

/// Data type for the extender code.
#[derive(Debug, Default, PartialEq, Clone, Copy)]
pub struct Extender([u8; 4]);

impl Extender {
    /// Fetch the extender code from the given source, while expecting it to exist.
    pub fn from_reader<S: Read>(mut source: S) -> Result<Self> {
        let mut extension = [0u8; 4];
        source.read_exact(&mut extension)?;
        Ok(extension.into())
    }

    /// Fetch the extender code from the given source, while
    /// being possible to not be available.
    /// Returns `None` if the source reaches EoF prematurely.
    /// Any other I/O error is delegated to a `NiftiError`.
    pub fn from_reader_optional<S: Read>(mut source: S) -> Result<Option<Self>> {
        let mut extension = [0u8; 4];
        match source.read_exact(&mut extension) {
            Ok(()) => Ok(Some(extension.into())),
            Err(ref e) if e.kind() == IoErrorKind::UnexpectedEof => Ok(None),
            Err(e) => Err(NiftiError::from(e)),
        }
    }

    /// Whether extensions should exist upon this extender code.
    pub fn has_extensions(&self) -> bool {
        self.0[0] != 0
    }

    /// Get the extender's bytes
    pub fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }
}

impl From<[u8; 4]> for Extender {
    fn from(extender: [u8; 4]) -> Self {
        Extender(extender)
    }
}

/// Data type for the raw contents of an extension.
/// Users of this type have to reinterpret the data
/// to suit their needs.
#[derive(Debug, PartialEq, Clone)]
pub struct Extension {
    esize: i32,
    ecode: i32,
    edata: Vec<u8>,
}

impl Extension {
    /// Create an extension out of its main components.
    ///
    /// # Panics
    /// If `esize` does not correspond to the full size
    /// of the extension in bytes: `8 + edata.len()`
    pub fn new(esize: i32, ecode: i32, edata: Vec<u8>) -> Self {
        if esize as usize != 8 + edata.len() {
            panic!(
                "Illegal extension size: esize is {}, but full size is {}",
                esize,
                edata.len()
            );
        }

        Extension {
            esize,
            ecode,
            edata,
        }
    }

    /// Create a new extension out of a &str
    pub fn from_str(ecode: i32, edata: &str) -> Self {
        let esize = 8 + edata.len() as i32;
        // pad the esize to a multiple of 16
        let padded_esize = (esize + 15) & !15;
        let mut edata = edata.as_bytes().to_vec();
        edata.resize(padded_esize as usize - 8, 0);
        Extension::new(padded_esize, ecode, edata)
    }

    /// Obtain the claimed extension raw size (`esize` field).
    pub fn size(&self) -> i32 {
        self.esize
    }

    /// Obtain the extension's code (`ecode` field).
    pub fn code(&self) -> i32 {
        self.ecode
    }

    /// Obtain the extension's data (`edata` field).
    pub fn data(&self) -> &Vec<u8> {
        &self.edata
    }

    /// Take the extension's raw data, discarding the rest.
    pub fn into_data(self) -> Vec<u8> {
        self.edata
    }
}

/// Data type for aggregating the extender code and
/// all extensions.
#[derive(Debug, PartialEq, Clone)]
pub struct ExtensionSequence {
    extender: Extender,
    extensions: Vec<Extension>,
}

impl IntoIterator for ExtensionSequence {
    type Item = Extension;
    type IntoIter = ::std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.extensions.into_iter()
    }
}

impl<'a> IntoIterator for &'a ExtensionSequence {
    type Item = &'a Extension;
    type IntoIter = ::std::slice::Iter<'a, Extension>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl ExtensionSequence {
    /// Provide a public constructor
    pub fn new(extender: Extender, extensions: Vec<Extension>) -> Self {
        ExtensionSequence {
            extender,
            extensions,
        }
    }

    /// Read a sequence of extensions from a source, up until `len` bytes.
    pub fn from_reader<S, E>(
        extender: Extender,
        mut source: ByteOrdered<S, E>,
        len: usize,
    ) -> Result<Self>
    where
        S: Read,
        E: Endian,
    {
        let mut extensions = Vec::new();
        if extender.has_extensions() {
            let mut offset = 0;
            while offset < len {
                let esize = source.read_i32()?;
                let ecode = source.read_i32()?;

                let data_size = (esize as usize).saturating_sub(8);
                let mut edata = Vec::new();
                edata
                    .try_reserve_exact(data_size)
                    .map_err(|e| NiftiError::ReserveExtended(data_size, e))?;
                let nb_bytes_written = (&mut source)
                    .take(data_size as u64)
                    .read_to_end(&mut edata)?;

                if nb_bytes_written != data_size {
                    return Err(NiftiError::IncompatibleLength(nb_bytes_written, data_size));
                }

                extensions.push(Extension::new(i32::max(esize, 8), ecode, edata));
                offset += esize as usize;
            }
        }

        Ok(ExtensionSequence {
            extender,
            extensions,
        })
    }

    /// Obtain an iterator to the extensions.
    pub fn iter(&self) -> ::std::slice::Iter<Extension> {
        self.extensions.iter()
    }

    /// Whether the sequence of extensions is empty.
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }

    /// Obtain the number of extensions available.
    pub fn len(&self) -> usize {
        self.extensions.len()
    }

    /// Return the number of bytes the extensions take on disk
    pub fn bytes_on_disk(&self) -> usize {
        self.extensions
            .iter()
            .map(|e| e.size() as usize)
            .sum::<usize>()
    }
    /// Get the extender code from this extension sequence.
    pub fn extender(&self) -> Extender {
        self.extender
    }
}
