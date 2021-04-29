use std::convert::TryInto;
use std::ffi::CString;
use std::os::raw::{c_char, c_int, c_void};
use std::slice;
use std::str;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

pub use crate::raw;
pub use crate::rediserror::RedisError;
pub use crate::redisvalue::RedisValue;

pub type RedisResult = Result<RedisValue, RedisError>;

pub const REDIS_OK: RedisResult = Ok(RedisValue::SimpleStringStatic("OK"));
pub const TYPE_METHOD_VERSION: u64 = raw::REDISMODULE_TYPE_METHOD_VERSION as u64;

pub trait NextArg {
    fn next_string(&mut self) -> Result<String, RedisError>;
    fn next_i64(&mut self) -> Result<i64, RedisError>;
    fn next_u64(&mut self) -> Result<u64, RedisError>;
    fn next_f64(&mut self) -> Result<f64, RedisError>;
    fn done(&mut self) -> Result<(), RedisError>;
}

impl<S, T> NextArg for T
where
    T: Iterator<Item = S>,
    S: AsRef<str>,
{
    fn next_string(&mut self) -> Result<String, RedisError> {
        self.next()
            .map_or(Err(RedisError::WrongArity), |v| Ok(v.as_ref().to_string()))
    }

    fn next_i64(&mut self) -> Result<i64, RedisError> {
        self.next()
            .map_or(Err(RedisError::WrongArity), |v| parse_integer(v.as_ref()))
    }

    fn next_u64(&mut self) -> Result<u64, RedisError> {
        self.next().map_or(Err(RedisError::WrongArity), |v| {
            parse_unsigned_integer(v.as_ref())
        })
    }

    fn next_f64(&mut self) -> Result<f64, RedisError> {
        self.next()
            .map_or(Err(RedisError::WrongArity), |v| parse_float(v.as_ref()))
    }

    /// Return an error if there are any more arguments
    fn done(&mut self) -> Result<(), RedisError> {
        self.next().map_or(Ok(()), |_| Err(RedisError::WrongArity))
    }
}

pub struct Args {
    bytestrings: Vec<Vec<u8>>
}

impl Args {
    pub fn from(bytestrings: Vec<Vec<u8>>) -> Self {
        Args { bytestrings }
    }

    pub fn from_redis_module_string(argv: *mut *mut raw::RedisModuleString,
                        argc: c_int,
    ) -> Result<Self, RedisError> {
        let mut vecs = Vec::<Vec<u8>>::new();
        for arg in unsafe { slice::from_raw_parts(argv, argc as usize) }.iter() {
            match RedisString::from_ptr(*arg) {
                Ok(v) => {
                    vecs.push(v.into());
                },
                Err(_) => return Err(RedisError::Str("UTF8 encoding error in handler args")),
            }
        }
        Ok(Args::from(vecs))
    }
}

impl Into<Vec<String>> for Args {
    fn into(self) -> Vec<String> {
        self.bytestrings.iter().map(|a| String::from_utf8_lossy(a).into_owned()).collect()
    }
}

impl Into<Vec<Vec<u8>>> for Args {
    fn into(self) -> Vec<Vec<u8>> {
        self.bytestrings
    }
}

pub fn parse_unsigned_integer(arg: &str) -> Result<u64, RedisError> {
    arg.parse()
        .map_err(|_| RedisError::String(format!("Couldn't parse as unsigned integer: {}", arg)))
}

pub fn parse_integer(arg: &str) -> Result<i64, RedisError> {
    arg.parse()
        .map_err(|_| RedisError::String(format!("Couldn't parse as integer: {}", arg)))
}

pub fn parse_float(arg: &str) -> Result<f64, RedisError> {
    arg.parse()
        .map_err(|_| RedisError::String(format!("Couldn't parse as float: {}", arg)))
}

///////////////////////////////////////////////////

#[derive(Debug)]
pub struct RedisString {
    ctx: *mut raw::RedisModuleCtx,
    pub inner: *mut raw::RedisModuleString,
}

impl RedisString {
    pub fn new(ctx: *mut raw::RedisModuleCtx, inner: *mut raw::RedisModuleString) -> RedisString {
        RedisString { ctx, inner }
    }

    pub fn create(ctx: *mut raw::RedisModuleCtx, s: &str) -> RedisString {
        let str = CString::new(s).unwrap();
        let inner = unsafe { raw::RedisModule_CreateString.unwrap()(ctx, str.as_ptr(), s.len()) };

        RedisString { ctx, inner }
    }

    pub fn from_ptr<'a>(ptr: *const raw::RedisModuleString) -> Result<&'a str, Utf8Error> {
        let mut len: libc::size_t = 0;
        let bytes = unsafe { raw::RedisModule_StringPtrLen.unwrap()(ptr, &mut len) };

        str::from_utf8(unsafe { slice::from_raw_parts(bytes as *const u8, len) })
    }

    pub fn append(&mut self, s: &str) -> raw::Status {
        raw::string_append_buffer(self.ctx, self.inner, s)
    }

    pub fn len(&self) -> usize {
        let mut len: usize = 0;
        raw::string_ptr_len(self.inner, &mut len);
        len
    }

    pub fn is_empty(&self) -> bool {
        let mut len: usize = 0;
        raw::string_ptr_len(self.inner, &mut len);
        len == 0
    }

    pub fn try_as_str(&self) -> Result<&str, Utf8Error> {
        Self::from_ptr(self.inner)
    }

    /// Performs lossy conversion of a `RedisString` into an owned `String. This conversion
    /// will replace any invalid UTF-8 sequences with U+FFFD REPLACEMENT CHARACTER, which
    /// looks like this: �.
    pub fn into_string_lossy(self) -> String {
        let mut len: libc::size_t = 0;
        let bytes = unsafe { raw::RedisModule_StringPtrLen.unwrap()(self.inner, &mut len) };
        let bytes = unsafe { slice::from_raw_parts(bytes as *const u8, len) };
        String::from_utf8_lossy(bytes).into_owned()
    }

    // TODO: Redis allows storing and retrieving any arbitrary bytes.
    // However rust's String and str can only store valid UTF-8.
    // Implement these to allow non-utf8 bytes to be consumed:
    // pub fn into_bytes(self) -> Vec<u8> {}
    // pub fn as_bytes(&self) -> &[u8] {}
    pub fn into_bytes(self) -> Vec<u8> {
        let mut len: libc::size_t = 0;
        let bytes = unsafe { raw::RedisModule_StringPtrLen.unwrap()(self.inner, &mut len) };
        // let bytes = unsafe { slice::from_raw_parts(bytes as *const u8, len) };
        unsafe { Vec::<u8>::from_raw_parts(bytes as *mut u8, len, len) }
    }

    pub fn as_bytes(&self) -> &[u8] {
        let mut len: libc::size_t = 0;
        let bytes = unsafe { raw::RedisModule_StringPtrLen.unwrap()(self.inner, &mut len) };
        let bytes = unsafe { slice::from_raw_parts(bytes as *const u8, len) };
        bytes
    }
}

impl Drop for RedisString {
    fn drop(&mut self) {
        unsafe {
            raw::RedisModule_FreeString.unwrap()(self.ctx, self.inner);
        }
    }
}

impl Clone for RedisString {
    fn clone(&self) -> RedisString {
        let inner =
            unsafe { raw::RedisModule_CreateStringFromString.unwrap()(self.ctx, self.inner) };
        RedisString::new(self.ctx, inner)
    }
}

impl From<RedisString> for String {
    fn from(rs: RedisString) -> Self {
        rs.into_string_lossy()
    }
}

///////////////////////////////////////////////////

#[derive(Debug)]
pub struct RedisBuffer {
    buffer: *mut c_char,
    len: usize,
}

impl RedisBuffer {
    pub fn new(buffer: *mut c_char, len: usize) -> RedisBuffer {
        RedisBuffer { buffer, len }
    }

    pub fn to_string(&self) -> Result<String, FromUtf8Error> {
        String::from_utf8(self.as_ref().to_vec())
    }
}

impl AsRef<[u8]> for RedisBuffer {
    fn as_ref(&self) -> &[u8] {
        unsafe { slice::from_raw_parts(self.buffer as *const u8, self.len) }
    }
}

impl Drop for RedisBuffer {
    fn drop(&mut self) {
        unsafe {
            raw::RedisModule_Free.unwrap()(self.buffer as *mut c_void);
        }
    }
}
