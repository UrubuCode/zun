//! Table section serialization.
//!
//! Each table is written as a `u32` row count followed by fixed-width rows
//! whose fields are all little-endian `u32`s (heap indices, raw tokens, RID
//! ranges). The six tables appear in a fixed order. Fixed-width rows keep the
//! reader trivial and leave room to move to the 2-vs-4-byte index compression
//! of ECMA-335 behind a minor version bump later.

use crate::ReadError;
use zun_metadata::{
    BlobIndex, FieldDef, MemberRef, MethodDef, ModuleRef, StringIndex, Tables, Token, TypeDef,
    TypeRef,
};

/// Serialize all tables into the tables section bytes.
pub fn write_tables(tables: &Tables) -> Vec<u8> {
    let mut out = Vec::new();

    write_table(&mut out, &tables.type_defs, |w, r| {
        w.extend([
            r.name.0,
            r.namespace.0,
            r.base.to_raw(),
            r.field_start,
            r.field_end,
            r.method_start,
            r.method_end,
        ]);
    });
    write_table(&mut out, &tables.field_defs, |w, r| {
        let kind = r.kind as u32 | (u32::from(r.is_static) << 7);
        w.extend([r.name.0, r.signature.0, kind]);
    });
    write_table(&mut out, &tables.method_defs, |w, r| {
        w.extend([r.name.0, r.signature.0, r.body_offset, u32::from(r.is_static)]);
    });
    write_table(&mut out, &tables.type_refs, |w, r| {
        w.extend([r.scope.to_raw(), r.name.0, r.namespace.0]);
    });
    write_table(&mut out, &tables.member_refs, |w, r| {
        w.extend([r.parent.to_raw(), r.name.0, r.signature.0]);
    });
    write_table(&mut out, &tables.module_refs, |w, r| {
        w.extend([r.name.0]);
    });

    out
}

/// Deserialize the tables section.
pub fn read_tables(bytes: &[u8]) -> Result<Tables, ReadError> {
    let mut r = Reader { bytes, pos: 0 };
    let mut tables = Tables::new();

    tables.type_defs = r.read_table(|r| {
        Ok(TypeDef {
            name: StringIndex(r.u32()?),
            namespace: StringIndex(r.u32()?),
            base: Token::from_raw(r.u32()?),
            field_start: r.u32()?,
            field_end: r.u32()?,
            method_start: r.u32()?,
            method_end: r.u32()?,
        })
    })?;
    tables.field_defs = r.read_table(|r| {
        let name = StringIndex(r.u32()?);
        let signature = BlobIndex(r.u32()?);
        let encoded_kind = r.u32()? as u8;
        Ok(FieldDef {
            name,
            signature,
            kind: encoded_kind & 0x7f,
            is_static: encoded_kind & 0x80 != 0,
        })
    })?;
    tables.method_defs = r.read_table(|r| {
        Ok(MethodDef {
            name: StringIndex(r.u32()?),
            signature: BlobIndex(r.u32()?),
            body_offset: r.u32()?,
            is_static: r.u32()? != 0,
        })
    })?;
    tables.type_refs = r.read_table(|r| {
        Ok(TypeRef {
            scope: Token::from_raw(r.u32()?),
            name: StringIndex(r.u32()?),
            namespace: StringIndex(r.u32()?),
        })
    })?;
    tables.member_refs = r.read_table(|r| {
        Ok(MemberRef {
            parent: Token::from_raw(r.u32()?),
            name: StringIndex(r.u32()?),
            signature: BlobIndex(r.u32()?),
        })
    })?;
    tables.module_refs = r.read_table(|r| {
        Ok(ModuleRef {
            name: StringIndex(r.u32()?),
        })
    })?;

    Ok(tables)
}

/// Write one table: row count, then each row via `encode_row` emitting `u32`s.
fn write_table<T>(out: &mut Vec<u8>, rows: &[T], encode_row: impl Fn(&mut U32Writer, &T)) {
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    let mut w = U32Writer { out };
    for row in rows {
        encode_row(&mut w, row);
    }
}

/// Emits `u32`s as little-endian bytes.
struct U32Writer<'a> {
    out: &'a mut Vec<u8>,
}

impl U32Writer<'_> {
    fn extend(&mut self, values: impl IntoIterator<Item = u32>) {
        for v in values {
            self.out.extend_from_slice(&v.to_le_bytes());
        }
    }
}

/// Cursor over the tables section.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn u32(&mut self) -> Result<u32, ReadError> {
        let end = self.pos + 4;
        let slice = self.bytes.get(self.pos..end).ok_or(ReadError::Truncated)?;
        self.pos = end;
        Ok(u32::from_le_bytes(slice.try_into().unwrap()))
    }

    fn read_table<T>(
        &mut self,
        decode_row: impl Fn(&mut Self) -> Result<T, ReadError>,
    ) -> Result<Vec<T>, ReadError> {
        let count = self.u32()? as usize;
        let mut rows = Vec::with_capacity(count.min(1 << 20));
        for _ in 0..count {
            rows.push(decode_row(self)?);
        }
        Ok(rows)
    }
}
