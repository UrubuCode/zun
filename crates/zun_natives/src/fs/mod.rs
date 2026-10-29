//! `zun:core` fs group — synchronous filesystem primitives backing the Node
//! `fs` library (the TS surface lives in the `zun-node` repository).
//!
//! These are the low-level host syscalls the `node:fs` wrapper composes into
//! the public API. They stay within the scalar/string native ABI: a native
//! returns `Str`/`Bool`/`I64`/`F64`/`Void`, never a managed value, so
//! `readdirSync` returns its entries NUL-joined (filenames cannot contain a NUL
//! byte) for the wrapper to split, and `statSync` is decomposed into scalar
//! getters. Buffer-returning reads land once the native boundary can hand back
//! a managed byte view.
//!
//! Errors are reported as [`NativeError`] with a Node-accurate message
//! (`"<CODE>: <description>, <syscall> '<path>'"`), so the wrapper can surface a
//! JS error whose `code`/`message` match Node's and satisfy its test suite.

use std::fs;
use std::io;
use std::path::Path;

use crate::{NativeArg, NativeError, NativeFn, NativeTy::*, NativeValue, Sink};

/// Map a `std::io::Error` to Node's `(code, description)` pair. Errno is
/// preferred (stable across platforms for the POSIX set); `ErrorKind` is the
/// fallback for errors the OS did not number.
fn error_code(err: &io::Error) -> (&'static str, &'static str) {
    // The common POSIX errno values Node surfaces by name.
    if let Some(errno) = err.raw_os_error() {
        match errno {
            1 => return ("EPERM", "operation not permitted"),
            2 => return ("ENOENT", "no such file or directory"),
            13 => return ("EACCES", "permission denied"),
            17 => return ("EEXIST", "file already exists"),
            20 => return ("ENOTDIR", "not a directory"),
            21 => return ("EISDIR", "illegal operation on a directory"),
            39 | 66 => return ("ENOTEMPTY", "directory not empty"),
            _ => {}
        }
    }
    match err.kind() {
        io::ErrorKind::NotFound => ("ENOENT", "no such file or directory"),
        io::ErrorKind::PermissionDenied => ("EACCES", "permission denied"),
        io::ErrorKind::AlreadyExists => ("EEXIST", "file already exists"),
        _ => ("EIO", "i/o error"),
    }
}

/// Build the Node-style error message for a failed `syscall` on `path`.
fn fs_error(err: &io::Error, syscall: &str, path: &str) -> NativeError {
    let (code, description) = error_code(err);
    NativeError(format!("{code}: {description}, {syscall} '{path}'"))
}

/// Build the two-path variant (`rename`, `copyfile`): `… , <syscall> '<a>' -> '<b>'`.
fn fs_error2(err: &io::Error, syscall: &str, a: &str, b: &str) -> NativeError {
    let (code, description) = error_code(err);
    NativeError(format!("{code}: {description}, {syscall} '{a}' -> '{b}'"))
}

/// `fs.existsSync(path)` — never throws; `false` for any error (matching Node).
fn exists_sync(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    Ok(NativeValue::Bool(Path::new(args[0].as_str()).exists()))
}

/// `fs.readFileSync(path, "utf8")` — the UTF-8 text read. Invalid UTF-8 is read
/// lossily; a byte-exact Buffer read is a follow-up (needs a managed return).
fn read_file_utf8(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    let bytes = fs::read(path).map_err(|e| fs_error(&e, "open", path))?;
    Ok(NativeValue::Str(String::from_utf8_lossy(&bytes).into_owned()))
}

/// `fs.writeFileSync(path, data)` — overwrite with UTF-8 text.
fn write_file_sync(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    fs::write(path, args[1].as_str()).map_err(|e| fs_error(&e, "open", path))?;
    Ok(NativeValue::Void)
}

/// `fs.appendFileSync(path, data)` — append UTF-8 text, creating the file.
fn append_file_sync(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    use std::io::Write;
    let path = args[0].as_str();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| fs_error(&e, "open", path))?;
    file.write_all(args[1].as_str().as_bytes())
        .map_err(|e| fs_error(&e, "write", path))?;
    Ok(NativeValue::Void)
}

/// `fs.mkdirSync(path, { recursive })` — the boolean carries `recursive`.
fn mkdir_sync(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    let recursive = matches!(args[1], NativeArg::Bool(true));
    let result = if recursive {
        fs::create_dir_all(path)
    } else {
        fs::create_dir(path)
    };
    result.map_err(|e| fs_error(&e, "mkdir", path))?;
    Ok(NativeValue::Void)
}

/// `fs.rmdirSync(path)` — remove an empty directory.
fn rmdir_sync(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    fs::remove_dir(path).map_err(|e| fs_error(&e, "rmdir", path))?;
    Ok(NativeValue::Void)
}

/// `fs.unlinkSync(path)` — remove a file.
fn unlink_sync(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    fs::remove_file(path).map_err(|e| fs_error(&e, "unlink", path))?;
    Ok(NativeValue::Void)
}

/// `fs.rmSync(path, { recursive, force })`: args `[path, recursive, force]`.
/// `force` swallows a not-found error, as in Node.
fn rm_sync(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    let recursive = matches!(args[1], NativeArg::Bool(true));
    let force = matches!(args[2], NativeArg::Bool(true));
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if force && err.kind() == io::ErrorKind::NotFound => {
            return Ok(NativeValue::Void)
        }
        Err(err) => return Err(fs_error(&err, "stat", path)),
    };
    let result = if metadata.is_dir() {
        if recursive {
            fs::remove_dir_all(path)
        } else {
            fs::remove_dir(path)
        }
    } else {
        fs::remove_file(path)
    };
    match result {
        Ok(()) => Ok(NativeValue::Void),
        Err(err) if force && err.kind() == io::ErrorKind::NotFound => Ok(NativeValue::Void),
        Err(err) => Err(fs_error(&err, "unlink", path)),
    }
}

/// `fs.renameSync(from, to)`.
fn rename_sync(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let (from, to) = (args[0].as_str(), args[1].as_str());
    fs::rename(from, to).map_err(|e| fs_error2(&e, "rename", from, to))?;
    Ok(NativeValue::Void)
}

/// `fs.copyFileSync(src, dest)`.
fn copy_file_sync(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let (src, dest) = (args[0].as_str(), args[1].as_str());
    fs::copy(src, dest).map_err(|e| fs_error2(&e, "copyfile", src, dest))?;
    Ok(NativeValue::Void)
}

/// `fs.readdirSync(path)` — directory entry names, NUL-joined for the wrapper to
/// split. A NUL byte cannot appear in a path component, so the join is lossless.
fn readdir_sync(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    let mut names = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| fs_error(&e, "scandir", path))? {
        let entry = entry.map_err(|e| fs_error(&e, "scandir", path))?;
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    Ok(NativeValue::Str(names.join("\u{0}")))
}

/// The entry kind of `path`'s metadata, as a short tag the wrapper maps to the
/// `Stats.isFile()`/`isDirectory()`/… predicates.
fn kind_tag(metadata: &fs::Metadata) -> &'static str {
    let file_type = metadata.file_type();
    if file_type.is_file() {
        "file"
    } else if file_type.is_dir() {
        "directory"
    } else if file_type.is_symlink() {
        "symlink"
    } else {
        "other"
    }
}

/// `fs.statSync(path).<kind>` — follows symlinks.
fn stat_kind(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    let metadata = fs::metadata(path).map_err(|e| fs_error(&e, "stat", path))?;
    Ok(NativeValue::Str(kind_tag(&metadata).to_string()))
}

/// `fs.lstatSync(path).<kind>` — does not follow symlinks.
fn lstat_kind(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    let metadata = fs::symlink_metadata(path).map_err(|e| fs_error(&e, "lstat", path))?;
    Ok(NativeValue::Str(kind_tag(&metadata).to_string()))
}

/// `fs.statSync(path).size` in bytes (follows symlinks).
fn stat_size(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    let metadata = fs::metadata(path).map_err(|e| fs_error(&e, "stat", path))?;
    Ok(NativeValue::Int(metadata.len() as i64))
}

/// `fs.statSync(path).mtimeMs` — last-modified time in milliseconds since the
/// Unix epoch, `0.0` if the platform cannot report it.
fn stat_mtime_ms(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    let metadata = fs::metadata(path).map_err(|e| fs_error(&e, "stat", path))?;
    let millis = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    Ok(NativeValue::Float(millis))
}

/// `fs.realpathSync(path)` — the canonical absolute path.
fn realpath_sync(args: &[NativeArg], _: &mut dyn Sink) -> Result<NativeValue, NativeError> {
    let path = args[0].as_str();
    let resolved = fs::canonicalize(path).map_err(|e| fs_error(&e, "realpath", path))?;
    Ok(NativeValue::Str(resolved.to_string_lossy().into_owned()))
}

/// The `zun:core` fs group. Appended last in [`MODULES`](crate::MODULES) so every
/// existing dispatch id stays stable.
pub const MODULE: crate::Module = crate::Module {
    name: "fs",
    functions: &[
        NativeFn { name: "fsExistsSync", params: &[Str], ret: Bool, imp: exists_sync },
        NativeFn { name: "fsReadFileUtf8", params: &[Str], ret: Str, imp: read_file_utf8 },
        NativeFn { name: "fsWriteFileSync", params: &[Str, Str], ret: Void, imp: write_file_sync },
        NativeFn { name: "fsAppendFileSync", params: &[Str, Str], ret: Void, imp: append_file_sync },
        NativeFn { name: "fsMkdirSync", params: &[Str, Bool], ret: Void, imp: mkdir_sync },
        NativeFn { name: "fsRmdirSync", params: &[Str], ret: Void, imp: rmdir_sync },
        NativeFn { name: "fsUnlinkSync", params: &[Str], ret: Void, imp: unlink_sync },
        NativeFn { name: "fsRmSync", params: &[Str, Bool, Bool], ret: Void, imp: rm_sync },
        NativeFn { name: "fsRenameSync", params: &[Str, Str], ret: Void, imp: rename_sync },
        NativeFn { name: "fsCopyFileSync", params: &[Str, Str], ret: Void, imp: copy_file_sync },
        NativeFn { name: "fsReaddirSync", params: &[Str], ret: Str, imp: readdir_sync },
        NativeFn { name: "fsStatKind", params: &[Str], ret: Str, imp: stat_kind },
        NativeFn { name: "fsLstatKind", params: &[Str], ret: Str, imp: lstat_kind },
        NativeFn { name: "fsStatSize", params: &[Str], ret: I64, imp: stat_size },
        NativeFn { name: "fsStatMtimeMs", params: &[Str], ret: F64, imp: stat_mtime_ms },
        NativeFn { name: "fsRealpathSync", params: &[Str], ret: Str, imp: realpath_sync },
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    struct NullSink;
    impl Sink for NullSink {
        fn println(&mut self, _: &str) {}
    }

    fn call(f: &NativeFn, args: &[NativeArg]) -> Result<NativeValue, NativeError> {
        (f.imp)(args, &mut NullSink)
    }

    fn find(name: &str) -> &'static NativeFn {
        MODULE.functions.iter().find(|f| f.name == name).expect("native present")
    }

    fn temp_dir() -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let unique = format!(
            "zun-fs-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        dir.push(unique);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_read_exists_and_remove_round_trip() {
        let dir = temp_dir();
        let file = dir.join("hello.txt");
        let path = file.to_str().unwrap();

        let write = find("fsWriteFileSync");
        let read = find("fsReadFileUtf8");
        let exists = find("fsExistsSync");
        let unlink = find("fsUnlinkSync");

        call(write, &[NativeArg::Str(path), NativeArg::Str("hi zun")]).unwrap();
        assert!(matches!(call(exists, &[NativeArg::Str(path)]).unwrap(), NativeValue::Bool(true)));
        match call(read, &[NativeArg::Str(path)]).unwrap() {
            NativeValue::Str(contents) => assert_eq!(contents, "hi zun"),
            other => panic!("expected text, got {other:?}"),
        }
        call(unlink, &[NativeArg::Str(path)]).unwrap();
        assert!(matches!(call(exists, &[NativeArg::Str(path)]).unwrap(), NativeValue::Bool(false)));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reading_a_missing_file_reports_node_enoent() {
        let missing = "./definitely-not-here-zun-fs.txt";
        let read = find("fsReadFileUtf8");
        let err = call(read, &[NativeArg::Str(missing)]).unwrap_err();
        assert!(
            err.0.starts_with("ENOENT: no such file or directory, open '"),
            "message should match Node's ENOENT open format, got {:?}",
            err.0
        );
    }

    #[test]
    fn mkdir_readdir_and_stat_kind() {
        let dir = temp_dir();
        let sub = dir.join("sub");
        let subpath = sub.to_str().unwrap();
        let file = dir.join("a.txt");

        call(find("fsMkdirSync"), &[NativeArg::Str(subpath), NativeArg::Bool(false)]).unwrap();
        call(find("fsWriteFileSync"), &[NativeArg::Str(file.to_str().unwrap()), NativeArg::Str("x")]).unwrap();

        match call(find("fsReaddirSync"), &[NativeArg::Str(dir.to_str().unwrap())]).unwrap() {
            NativeValue::Str(joined) => {
                let mut names: Vec<&str> = joined.split('\u{0}').collect();
                names.sort_unstable();
                assert_eq!(names, vec!["a.txt", "sub"]);
            }
            other => panic!("expected joined names, got {other:?}"),
        }

        match call(find("fsStatKind"), &[NativeArg::Str(subpath)]).unwrap() {
            NativeValue::Str(kind) => assert_eq!(kind, "directory"),
            other => panic!("expected kind, got {other:?}"),
        }
        match call(find("fsStatSize"), &[NativeArg::Str(file.to_str().unwrap())]).unwrap() {
            NativeValue::Int(size) => assert_eq!(size, 1),
            other => panic!("expected size, got {other:?}"),
        }

        fs::remove_dir_all(&dir).ok();
    }
}
