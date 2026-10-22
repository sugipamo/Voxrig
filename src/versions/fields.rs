//! Bounded wire primitives shared by adapters; packet layouts and NBT stay version-specific.
use anyhow::{Context, Result, bail};

pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
}
impl<'a> Reader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
    pub(crate) fn remaining(&self) -> &'a [u8] {
        self.bytes
    }
    pub fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        if length > self.bytes.len() {
            bail!("truncated packet field");
        }
        let (field, rest) = self.bytes.split_at(length);
        self.bytes = rest;
        Ok(field)
    }
    pub fn end(&self) -> Result<()> {
        if !self.bytes.is_empty() {
            bail!(
                "unexpected trailing packet data: {} bytes",
                self.bytes.len()
            );
        }
        Ok(())
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => bail!("invalid packet boolean"),
        }
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into()?))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into()?))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into()?))
    }
    pub fn f32(&mut self) -> Result<f32> {
        let value = f32::from_be_bytes(self.take(4)?.try_into()?);
        if !value.is_finite() {
            bail!("non-finite packet float");
        }
        Ok(value)
    }
    pub fn f64(&mut self) -> Result<f64> {
        let value = f64::from_be_bytes(self.take(8)?.try_into()?);
        if !value.is_finite() {
            bail!("non-finite packet double");
        }
        Ok(value)
    }
    pub fn varint(&mut self) -> Result<i32> {
        crate::protocol::get_varint(&mut self.bytes)
    }
    pub fn count(&mut self, maximum: usize) -> Result<usize> {
        let count = usize::try_from(self.varint()?).context("negative packet count")?;
        if count > maximum {
            bail!("packet count exceeds limit {maximum}");
        }
        Ok(count)
    }
    pub fn string(&mut self) -> Result<String> {
        crate::protocol::get_string(&mut self.bytes)
    }
    pub fn byte_array(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let length = self.count(maximum)?;
        self.take(length)
    }
}
