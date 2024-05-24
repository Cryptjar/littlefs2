//! Paths

use core::{
    cmp::Ordering,
    convert::TryFrom,
    ffi::{c_char, CStr},
    fmt,
    iter::FusedIterator,
    ops, ptr, slice, str,
};

use crate::consts;

/// A path
///
/// Paths must be null terminated ASCII strings
///
/// This assumption is not needed for littlefs itself (it works like Linux and
/// accepts arbitrary C strings), but the assumption makes `AsRef<str>` trivial
/// to implement.
#[derive(PartialEq, Eq)]
#[repr(transparent)]
pub struct Path {
    inner: CStr,
}

impl Path {
    /// Compare the path using their string representation
    /// This comarison function as would be expected for a `String` type.
    ///
    /// <div class="warning">
    ///   This ordering does not match the ordering obsvered when iterating over a directory.
    ///
    ///   See <a href="#method.cmp_lfs">cmp_lfs</a> and <a href = "https://github.com/littlefs-project/littlefs/issues/923">littlefs#923</a>.
    /// </div>
    ///
    /// ```
    ///# use std::cmp::Ordering;
    ///# use littlefs2::path;
    /// assert_eq!(path!("some_path_a").cmp_str(path!("some_path_b")), Ordering::Less);
    /// assert_eq!(path!("some_path_b").cmp_str(path!("some_path_a")), Ordering::Greater);
    /// assert_eq!(path!("some_path").cmp_str(path!("some_path_a")), Ordering::Less);
    /// assert_eq!(path!("some_path").cmp_str(path!("some_path_b")), Ordering::Less);
    /// assert_eq!(path!("some_path").cmp_str(path!("some_path")), Ordering::Equal);
    ///```
    pub fn cmp_str(&self, other: &Path) -> Ordering {
        self.inner.cmp(&other.inner)
    }

    /// Compare the path using their string representation
    ///
    /// This comparison function matches the iteration order of `littlefs` when iterating over directory.
    /// For more information, see [littlefs#923](https://github.com/littlefs-project/littlefs/issues/923)
    ///
    /// ```
    ///# use std::cmp::Ordering;
    ///# use littlefs2::path;
    /// assert_eq!(path!("some_path_a").cmp_lfs(path!("some_path_b")), Ordering::Less);
    /// assert_eq!(path!("some_path_b").cmp_lfs(path!("some_path_a")), Ordering::Greater);
    /// assert_eq!(path!("some_path").cmp_lfs(path!("some_path_a")), Ordering::Greater);
    /// assert_eq!(path!("some_path").cmp_lfs(path!("some_path_b")), Ordering::Greater);
    /// assert_eq!(path!("some_path_a").cmp_lfs(path!("some_path")), Ordering::Less);
    /// assert_eq!(path!("some_path_b").cmp_lfs(path!("some_path")), Ordering::Less);
    /// assert_eq!(path!("some_path").cmp_lfs(path!("some_path")), Ordering::Equal);
    ///```
    pub fn cmp_lfs(&self, other: &Path) -> Ordering {
        let this = self.inner.to_bytes();
        let other = other.inner.to_bytes();

        let min_len = this.len().min(other.len());

        match this[0..min_len].cmp(&other[0..min_len]) {
            // if they have a clear ordering, return this ordering
            Ordering::Less => Ordering::Less,
            // if they have a clear ordering, return this ordering
            Ordering::Greater => Ordering::Greater,
            // If one is a prefix of the other, the longest on is the first
            Ordering::Equal => other.len().cmp(&this.len()),
        }
    }
}

/// Iterator over the ancestors of a Path
///
/// See documentation for [`Path::ancestors`][]
pub struct Ancestors<'a> {
    path: &'a str,
}

impl<'a> Iterator for Ancestors<'a> {
    type Item = PathBuf;
    fn next(&mut self) -> Option<PathBuf> {
        if self.path.is_empty() {
            return None;
        } else if self.path == "/" {
            self.path = "";
            return Some("/".into());
        }

        let item = self.path;

        let Some((rem, item_name)) = self.path.rsplit_once('/') else {
            self.path = "";
            return Some(item.into());
        };

        if self.path.starts_with('/') && rem.is_empty() {
            self.path = "/";
        } else {
            self.path = rem;
        }

        // Case of a path ending with a trailing `/`
        if item_name.is_empty() {
            self.next();
        }
        Some(item.into())
    }
}

impl<'a> FusedIterator for Ancestors<'a> {}

/// Iterator over the components of a Path
///
/// See documentation for [`Path::iter`][]
pub struct Iter<'a> {
    path: &'a str,
}

impl<'a> Iterator for Iter<'a> {
    type Item = PathBuf;
    fn next(&mut self) -> Option<PathBuf> {
        if self.path.is_empty() {
            return None;
        }
        if self.path.starts_with('/') {
            self.path = &self.path[1..];
            return Some("/".into());
        }

        let Some((path, rem)) = self.path.split_once('/') else {
            let ret_val = Some(self.path.into());
            self.path = "";
            return ret_val;
        };

        self.path = rem;
        Some(path.into())
    }
}

impl Path {
    /// Return true if the path is empty
    ///
    /// ```rust
    ///# use littlefs2::path;
    ///
    /// assert!(path!("").is_empty());
    /// assert!(!path!("something").is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.inner.to_bytes().is_empty()
    }

    /// Get the name of the file this path points to if it points to one
    ///
    /// ```
    ///# use littlefs2::path;
    /// let path = path!("/some/path/file.extension");
    /// assert_eq!(path.file_name(), Some(path!("file.extension")));
    ///
    /// let path = path!("/");
    /// assert_eq!(path.file_name(), None);
    ///
    /// let path = path!("");
    /// assert_eq!(path.file_name(), None);
    ///
    /// let path = path!("/some/path/file.extension/");
    /// assert_eq!(path.file_name(), None);
    /// ```
    pub fn file_name(&self) -> Option<&Path> {
        if self.is_empty() {
            return None;
        }

        let this = self.as_str_ref_with_trailing_nul();
        match this.rsplit_once('/') {
            None | Some((_, "\x00")) => None,
            Some((_, path)) => {
                debug_assert!(path.ends_with('\x00'));
                Some(unsafe { Path::from_bytes_with_nul_unchecked(path.as_bytes()) })
            }
        }
    }

    /// Iterate over the ancestors of the path
    ///
    /// ```
    ///# use littlefs2::path;
    /// let path = path!("/some/path/file.extension");
    /// let mut ancestors = path.ancestors();
    /// assert_eq!(&*ancestors.next().unwrap(), path!("/some/path/file.extension"));
    /// assert_eq!(&*ancestors.next().unwrap(), path!("/some/path"));
    /// assert_eq!(&*ancestors.next().unwrap(), path!("/some"));
    /// assert_eq!(&*ancestors.next().unwrap(), path!("/"));
    /// assert!(ancestors.next().is_none());
    /// ```
    pub fn ancestors(&self) -> Ancestors {
        Ancestors {
            path: self.as_str(),
        }
    }

    /// Iterate over the components of the path
    ///
    /// ```
    ///# use littlefs2::path;
    /// let path = path!("/some/path/file.extension");
    /// let mut iter = path.iter();
    /// assert_eq!(&*iter.next().unwrap(), path!("/"));
    /// assert_eq!(&*iter.next().unwrap(), path!("some"));
    /// assert_eq!(&*iter.next().unwrap(), path!("path"));
    /// assert_eq!(&*iter.next().unwrap(), path!("file.extension"));
    /// assert!(iter.next().is_none());
    /// ```
    pub fn iter(&self) -> Iter {
        Iter {
            path: self.as_str(),
        }
    }

    /// Creates a path from a string.
    ///
    /// The string must only consist of ASCII characters, expect for the last character which must
    /// be null.  If these conditions are not met, this function panics.
    pub const fn from_str_with_nul(s: &str) -> &Self {
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len().saturating_sub(1) {
            assert!(bytes[i] != 0, "must not contain null");
            assert!(bytes[i].is_ascii(), "must be ASCII");
            i += 1;
        }
        assert!(!bytes.is_empty(), "must not be empty");
        assert!(bytes[i] == 0, "last byte must be null");
        unsafe { Self::from_bytes_with_nul_unchecked(bytes) }
    }

    /// Creates a path from a byte buffer
    ///
    /// The buffer will be first interpreted as a `CStr` and then checked to be comprised only of
    /// ASCII characters.
    pub fn from_bytes_with_nul(bytes: &[u8]) -> Result<&Self> {
        let cstr = CStr::from_bytes_with_nul(bytes).map_err(|_| Error::NotCStr)?;
        Self::from_cstr(cstr)
    }

    /// Unchecked version of `from_bytes_with_nul`
    ///
    /// # Safety
    /// `bytes` must be null terminated string comprised of only ASCII characters
    pub const unsafe fn from_bytes_with_nul_unchecked(bytes: &[u8]) -> &Self {
        &*(bytes as *const [u8] as *const Path)
    }

    /// Creates a path from a C string
    ///
    /// The string will be checked to be comprised only of ASCII characters
    // XXX should we reject empty paths (`""`) here?
    pub fn from_cstr(cstr: &CStr) -> Result<&Self> {
        let bytes = cstr.to_bytes();
        let n = cstr.to_bytes().len();
        if n > consts::PATH_MAX {
            Err(Error::TooLarge)
        } else if bytes.is_ascii() {
            Ok(unsafe { Self::from_cstr_unchecked(cstr) })
        } else {
            Err(Error::NotAscii)
        }
    }

    /// Unchecked version of `from_cstr`
    ///
    /// # Safety
    /// `cstr` must be comprised only of ASCII characters
    pub unsafe fn from_cstr_unchecked(cstr: &CStr) -> &Self {
        &*(cstr as *const CStr as *const Path)
    }

    /// Returns the inner pointer to this C string.
    pub(crate) fn as_ptr(&self) -> *const c_char {
        self.inner.as_ptr()
    }

    /// Creates an owned `PathBuf` with `path` adjoined to `self`.
    pub fn join(&self, path: &Path) -> PathBuf {
        let mut p = PathBuf::from(self);
        p.push(path);
        p
    }

    pub fn exists<S: crate::driver::Storage>(&self, fs: &crate::fs::Filesystem<S>) -> bool {
        fs.metadata(self).is_ok()
    }

    // helpful for debugging wither the trailing nul is indeed a trailing nul.
    pub fn as_str_ref_with_trailing_nul(&self) -> &str {
        // SAFETY: ASCII is valid UTF-8
        unsafe { str::from_utf8_unchecked(self.inner.to_bytes_with_nul()) }
    }

    pub fn as_str(&self) -> &str {
        // SAFETY: ASCII is valid UTF-8
        unsafe { str::from_utf8_unchecked(self.inner.to_bytes()) }
    }

    pub fn parent(&self) -> Option<PathBuf> {
        let rk_path_bytes = self.as_ref()[..].as_bytes();
        match rk_path_bytes.iter().rposition(|x| *x == b'/') {
            Some(0) if rk_path_bytes.len() != 1 => Some(PathBuf::from("/")),
            Some(slash_index) => {
                // if we have a directory that ends with `/`,
                // still need to "go up" one parent
                if slash_index + 1 == rk_path_bytes.len() {
                    PathBuf::from(&rk_path_bytes[..slash_index]).parent()
                } else {
                    Some(PathBuf::from(&rk_path_bytes[..slash_index]))
                }
            }
            None => None,
        }
    }
}

impl AsRef<str> for Path {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // helpful for debugging wither the trailing nul is indeed a trailing nul.
        write!(f, "p{:?}", self.as_str_ref_with_trailing_nul())
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

impl<'b> TryFrom<&'b [u8]> for &'b Path {
    type Error = Error;

    fn try_from(bytes: &[u8]) -> Result<&Path> {
        Path::from_bytes_with_nul(bytes)
    }
}

impl PartialEq<str> for Path {
    fn eq(&self, rhs: &str) -> bool {
        self.as_ref() == rhs
    }
}

// without this you need to slice byte string literals (`b"foo\0"[..].try_into()`)
macro_rules! array_impls {
    ($($N:expr),+) => {
        $(
            impl<'b> TryFrom<&'b [u8; $N]> for &'b Path {
                type Error = Error;

                fn try_from(bytes: &[u8; $N]) -> Result<&Path> {
                    Path::from_bytes_with_nul(&bytes[..])
                }
            }

            impl From<&[u8; $N]> for PathBuf {
                fn from(bytes: &[u8; $N]) -> Self {
                    Self::from(&bytes[..])
                }
            }

            impl PartialEq<[u8; $N]> for Path {
                fn eq(&self, rhs: &[u8; $N]) -> bool {
                    self.as_ref().as_bytes() == &rhs[..]
                }
            }

        )+
    }
}

array_impls!(
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32
);

/// An owned, mutable path
#[derive(Clone)]
pub struct PathBuf {
    buf: [c_char; consts::PATH_MAX_PLUS_ONE],
    // NOTE `len` DOES include the final null byte
    len: usize,
}

/// # Safety
/// `s` must point to valid memory; `s` will be treated as a null terminated string
unsafe fn strlen(mut s: *const c_char) -> usize {
    let mut n = 0;
    while *s != 0 {
        s = s.add(1);
        n += 1;
    }
    n
}

impl Default for PathBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl PathBuf {
    pub fn new() -> Self {
        Self {
            buf: [0; consts::PATH_MAX_PLUS_ONE],
            len: 1,
        }
    }

    pub fn clear(&mut self) {
        self.buf = [0; consts::PATH_MAX_PLUS_ONE];
        self.len = 1;
    }

    pub(crate) unsafe fn from_buffer(buf: [c_char; consts::PATH_MAX_PLUS_ONE]) -> Self {
        let len = strlen(buf.as_ptr()) + 1 /* null byte */;
        PathBuf { buf, len }
    }

    /// Extends `self` with `path`
    pub fn push(&mut self, path: &Path) {
        match path.as_ref() {
            // no-operation
            "" => return,

            // `self` becomes `/` (root), to match `std::Path` implementation
            // NOTE(allow) cast is necessary on some architectures (e.g. x86)
            #[allow(clippy::unnecessary_cast)]
            "/" => {
                self.buf[0] = b'/' as c_char;
                self.buf[1] = 0;
                self.len = 2;
                return;
            }
            _ => {}
        }

        let src = path.as_ref().as_bytes();
        let needs_separator = self
            .as_ref()
            .as_bytes()
            .last()
            .map(|byte| *byte != b'/')
            .unwrap_or(false);
        let slen = src.len();
        #[cfg(test)]
        println!("{}, {}, {}", self.len, slen, consts::PATH_MAX_PLUS_ONE);
        // hprintln!("{}, {}, {}", self.len, slen, consts::PATH_MAX_PLUS_ONE);
        assert!(
            self.len
                + slen
                + if needs_separator {
                    // b'/'
                    1
                } else {
                    0
                }
                <= consts::PATH_MAX_PLUS_ONE
        );

        let len = self.len;
        unsafe {
            let mut p = self.buf.as_mut_ptr().cast::<u8>().add(len - 1);
            if needs_separator {
                p.write(b'/');
                p = p.add(1);
                self.len += 1;
            }
            ptr::copy_nonoverlapping(src.as_ptr(), p, slen);
            p.add(slen).write(0); // null byte
            self.len += slen;
        }
    }
}

impl From<&Path> for PathBuf {
    fn from(path: &Path) -> Self {
        let bytes = path.as_ref().as_bytes();

        let mut buf = [0; consts::PATH_MAX_PLUS_ONE];
        let len = bytes.len();
        assert!(len <= consts::PATH_MAX);
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), buf.as_mut_ptr().cast(), len + 1) }
        Self { buf, len: len + 1 }
    }
}

impl From<&[u8]> for PathBuf {
    /// Accepts byte string, with or without trailing nul.
    ///
    /// PANICS: when there are embedded nuls
    fn from(bytes: &[u8]) -> Self {
        // NB: This needs to set the final NUL byte, unless it already has one
        // It also checks that there are no inner NUL bytes
        let bytes = if !bytes.is_empty() && bytes[bytes.len() - 1] == b'\0' {
            &bytes[..bytes.len() - 1]
        } else {
            bytes
        };
        let has_no_embedded_nul = !bytes.iter().any(|&byte| byte == b'\0');
        assert!(has_no_embedded_nul);

        let mut buf = [0; consts::PATH_MAX_PLUS_ONE];
        let len = bytes.len();
        assert!(len <= consts::PATH_MAX);
        assert!(bytes.is_ascii());
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), buf.as_mut_ptr().cast(), len) }
        Self { buf, len: len + 1 }
    }
}

impl From<&str> for PathBuf {
    fn from(s: &str) -> Self {
        PathBuf::from(s.as_bytes())
    }
}

impl ops::Deref for PathBuf {
    type Target = Path;

    fn deref(&self) -> &Path {
        unsafe {
            Path::from_bytes_with_nul_unchecked(slice::from_raw_parts(
                self.buf.as_ptr().cast(),
                self.len,
            ))
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for PathBuf {
    fn serialize<S>(&self, serializer: S) -> core::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bytes(self.as_ref().as_bytes())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for PathBuf {
    fn deserialize<D>(deserializer: D) -> core::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use core::marker::PhantomData;

        struct ValueVisitor<'de>(PhantomData<&'de ()>);

        impl<'de> serde::de::Visitor<'de> for ValueVisitor<'de> {
            type Value = PathBuf;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a path buffer")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> core::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if v.len() > consts::PATH_MAX {
                    return Err(E::invalid_length(v.len(), &self));
                }
                Ok(PathBuf::from(v))
            }
        }

        deserializer.deserialize_bytes(ValueVisitor(PhantomData))
    }
}

impl fmt::Debug for PathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Path as fmt::Debug>::fmt(self, f)
    }
}

impl fmt::Display for PathBuf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Path as fmt::Display>::fmt(self, f)
    }
}

impl core::cmp::PartialEq for PathBuf {
    fn eq(&self, other: &Self) -> bool {
        // from cstr_core
        self.as_ref() == other.as_ref()

        // // use cortex_m_semihosting::hprintln;
        // // hprintln!("inside PathBuf PartialEq");
        // // hprintln!("self.len {}, other.len {}", self.len, other.len).ok();
        // // hprintln!("self..len {:?}, other..len {:?}", &self.buf[..self.len], &other.buf[..other.len]).ok();
        // self.len == other.len && self.buf[..self.len - 1] == other.buf[..other.len - 1]
    }
}

impl core::cmp::Eq for PathBuf {}

// use core::cmp::Ordering;

// impl Ord for PathBuf {
//     fn cmp(&self, other: &Self) -> Ordering {
//         self.len.cmp(&other.len)
//     }
// }

// impl PartialOrd for PathBuf {
//     fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
//         Some(self.cmp(other))
//     }
// }

/// Errors that arise from converting byte buffers into paths
#[derive(Clone, Copy, Debug)]
pub enum Error {
    /// Byte buffer contains non-ASCII characters
    NotAscii,
    /// Byte buffer is not a C string
    NotCStr,
    /// Byte buffer is too long (longer than `consts::PATH_MAX_PLUS_ONE`)
    TooLarge,
}

/// Result type that has its Error variant set to `path::Error`
pub type Result<T> = core::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::{Path, PathBuf};
    use crate::path;

    const EMPTY: &Path = path!("");
    const SLASH: &Path = path!("/");

    #[test]
    fn path_macro() {
        assert_eq!(EMPTY, &*PathBuf::from(""));
        assert_eq!(SLASH, &*PathBuf::from("/"));
    }

    // does not compile:
    // const NON_ASCII: &Path = path!("über");
    // const NULL: &Path = path!("ub\0er");

    #[test]
    #[should_panic]
    fn nul_in_from_str_with_nul() {
        Path::from_str_with_nul("ub\0er");
    }

    #[test]
    #[should_panic]
    fn non_ascii_in_from_str_with_nul() {
        Path::from_str_with_nul("über");
    }

    #[test]
    fn join() {
        let empty = Path::from_bytes_with_nul(b"\0").unwrap();
        let slash = Path::from_bytes_with_nul(b"/\0").unwrap();
        let a = Path::from_bytes_with_nul(b"a\0").unwrap();
        let b = Path::from_bytes_with_nul(b"b\0").unwrap();

        assert_eq!(empty.join(empty).as_ref(), "");
        assert_eq!(empty.join(slash).as_ref(), "/");
        assert_eq!(empty.join(a).as_ref(), "a");
        assert_eq!(empty.join(b).as_ref(), "b");

        assert_eq!(slash.join(empty).as_ref(), "/");
        assert_eq!(slash.join(slash).as_ref(), "/");
        assert_eq!(slash.join(a).as_ref(), "/a");
        assert_eq!(slash.join(b).as_ref(), "/b");

        assert_eq!(a.join(empty).as_ref(), "a");
        assert_eq!(a.join(slash).as_ref(), "/");
        assert_eq!(a.join(a).as_ref(), "a/a");
        assert_eq!(a.join(b).as_ref(), "a/b");

        assert_eq!(b.join(empty).as_ref(), "b");
        assert_eq!(b.join(slash).as_ref(), "/");
        assert_eq!(b.join(a).as_ref(), "b/a");
        assert_eq!(b.join(b).as_ref(), "b/b");
    }

    #[test]
    fn nulls() {
        assert!(Path::from_bytes_with_nul(b"abc\0def").is_err());
    }

    #[test]
    fn trailing_nuls() {
        assert_eq!(PathBuf::from("abc"), PathBuf::from("abc\0"));
    }

    #[test]
    fn ancestors() {
        fn assert_ancestor_parent(path: &Path) {
            let mut ancestors = path.ancestors();
            if !path.as_str().is_empty() {
                assert_eq!(&*ancestors.next().unwrap(), path);
            }
            let mut buf = PathBuf::from(path);
            loop {
                let parent = buf.parent();
                assert_eq!(parent, ancestors.next());
                match parent {
                    Some(p) => buf = p,
                    None => return,
                }
            }
        }

        let path = path!("/some/path/.././file.extension");
        assert_ancestor_parent(path);
        let mut ancestors = path.ancestors();
        assert_eq!(
            &*ancestors.next().unwrap(),
            path!("/some/path/.././file.extension")
        );
        assert_eq!(&*ancestors.next().unwrap(), path!("/some/path/../."));
        assert_eq!(&*ancestors.next().unwrap(), path!("/some/path/.."));
        assert_eq!(&*ancestors.next().unwrap(), path!("/some/path"));
        assert_eq!(&*ancestors.next().unwrap(), path!("/some"));
        assert_eq!(&*ancestors.next().unwrap(), path!("/"));
        assert!(ancestors.next().is_none());

        let path = path!("/some/path/.././file.extension/");
        assert_ancestor_parent(path);
        let mut ancestors = path.ancestors();
        assert_eq!(
            &*ancestors.next().unwrap(),
            path!("/some/path/.././file.extension/")
        );
        assert_eq!(&*ancestors.next().unwrap(), path!("/some/path/../."));
        assert_eq!(&*ancestors.next().unwrap(), path!("/some/path/.."));
        assert_eq!(&*ancestors.next().unwrap(), path!("/some/path"));
        assert_eq!(&*ancestors.next().unwrap(), path!("/some"));
        assert_eq!(&*ancestors.next().unwrap(), path!("/"));
        assert!(ancestors.next().is_none());

        let path = path!("some/path/.././file.extension");
        assert_ancestor_parent(path);
        let mut ancestors = path.ancestors();
        assert_eq!(
            &*ancestors.next().unwrap(),
            path!("some/path/.././file.extension")
        );
        assert_eq!(&*ancestors.next().unwrap(), path!("some/path/../."));
        assert_eq!(&*ancestors.next().unwrap(), path!("some/path/.."));
        assert_eq!(&*ancestors.next().unwrap(), path!("some/path"));
        assert_eq!(&*ancestors.next().unwrap(), path!("some"));
        assert!(ancestors.next().is_none());
    }

    #[test]
    fn iter() {
        let path = path!("/some/path/.././file.extension");
        let mut ancestors = path.iter();
        assert_eq!(&*ancestors.next().unwrap(), path!("/"));
        assert_eq!(&*ancestors.next().unwrap(), path!("some"));
        assert_eq!(&*ancestors.next().unwrap(), path!("path"));
        assert_eq!(&*ancestors.next().unwrap(), path!(".."));
        assert_eq!(&*ancestors.next().unwrap(), path!("."));
        assert_eq!(&*ancestors.next().unwrap(), path!("file.extension"));
        assert!(ancestors.next().is_none());
        let path = path!("some/path/.././file.extension/");
        let mut ancestors = path.iter();
        assert_eq!(&*ancestors.next().unwrap(), path!("some"));
        assert_eq!(&*ancestors.next().unwrap(), path!("path"));
        assert_eq!(&*ancestors.next().unwrap(), path!(".."));
        assert_eq!(&*ancestors.next().unwrap(), path!("."));
        assert_eq!(&*ancestors.next().unwrap(), path!("file.extension"));
        assert!(ancestors.next().is_none());
    }

    #[test]
    fn file_name() {
        let path = path!("/some/path/.././file.extension");
        assert_eq!(path.file_name(), Some(path!("file.extension")));

        let path = path!("/");
        assert_eq!(path.file_name(), None);

        let path = path!("");
        assert_eq!(path.file_name(), None);

        let path = path!("/some/path/.././file.extension/");
        assert_eq!(path.file_name(), None);
    }
}
