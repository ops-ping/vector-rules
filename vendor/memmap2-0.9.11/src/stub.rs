use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

pub struct MmapInner {
    data: Box<[u8]>,
}

impl MmapInner {
    fn unsupported() -> io::Result<MmapInner> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn read(len: usize, file: &File, offset: u64) -> io::Result<MmapInner> {
        let file_len = file.metadata()?.len();
        let end = offset
            .checked_add(len as u64)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        if end > file_len {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let mut data = vec![0; len];
        let mut reader = file;
        reader.seek(SeekFrom::Start(offset))?;
        reader.read_exact(&mut data)?;
        Ok(MmapInner {
            data: data.into_boxed_slice(),
        })
    }

    pub fn map(len: usize, file: &File, offset: u64, _: bool, _: bool) -> io::Result<MmapInner> {
        MmapInner::read(len, file, offset)
    }

    pub fn map_exec(_: usize, _: &File, _: u64, _: bool, _: bool) -> io::Result<MmapInner> {
        MmapInner::unsupported()
    }

    pub fn map_mut(_: usize, _: &File, _: u64, _: bool, _: bool) -> io::Result<MmapInner> {
        MmapInner::unsupported()
    }

    pub fn map_copy(_: usize, _: &File, _: u64, _: bool, _: bool) -> io::Result<MmapInner> {
        MmapInner::unsupported()
    }

    pub fn map_copy_read_only(
        len: usize,
        file: &File,
        offset: u64,
        _: bool,
        _: bool,
    ) -> io::Result<MmapInner> {
        MmapInner::read(len, file, offset)
    }

    pub fn map_anon(_: usize, _: bool, _: bool, _: Option<u8>, _: bool) -> io::Result<MmapInner> {
        MmapInner::unsupported()
    }

    pub fn flush(&self, _: usize, _: usize) -> io::Result<()> {
        Ok(())
    }

    pub fn flush_async(&self, _: usize, _: usize) -> io::Result<()> {
        Ok(())
    }

    pub fn make_read_only(&mut self) -> io::Result<()> {
        Ok(())
    }

    pub fn make_exec(&mut self) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    pub fn make_mut(&mut self) -> io::Result<()> {
        Err(io::ErrorKind::Unsupported.into())
    }

    #[inline]
    pub fn ptr(&self) -> *const u8 {
        self.data.as_ptr()
    }

    #[inline]
    pub fn mut_ptr(&mut self) -> *mut u8 {
        self.data.as_mut_ptr()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.data.len()
    }
}

pub fn file_len(file: &File) -> io::Result<u64> {
    Ok(file.metadata()?.len())
}
