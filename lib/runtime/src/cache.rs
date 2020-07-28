//! The cache module provides the common data structures used by compiler backends to allow
//! serializing compiled wasm code to a binary format.  The binary format can be persisted,
//! and loaded to allow skipping compilation and fast startup.

use crate::Module;
use memmap::Mmap;
use std::{
    fmt,
    fs::{create_dir_all, File},
    io::{self, Write},
    path::PathBuf,
};

pub use super::Backend;
use wasmer_runtime_core_fl::cache::Error as CacheError;
pub use wasmer_runtime_core_fl::cache::{Artifact, WasmHash};

/// A generic cache for storing and loading compiled wasm modules.
///
/// The `wasmer-runtime` supplies a naive `FileSystemCache` api.
pub trait Cache {
    /// Error type to return when load error occurs
    type LoadError: fmt::Debug;
    /// Error type to return when store error occurs
    type StoreError: fmt::Debug;

    /// loads a module using the default `Backend`
    fn load(&self, key: WasmHash) -> Result<Module, Self::LoadError>;
    /// loads a cached module using a specific `Backend`
    fn load_with_backend(&self, key: WasmHash, backend: Backend)
        -> Result<Module, Self::LoadError>;
    /// Store a module into the cache with the given key
    fn store(&mut self, key: WasmHash, module: Module) -> Result<(), Self::StoreError>;
}

/// Representation of a directory that contains compiled wasm artifacts.
///
/// The `FileSystemCache` type implements the [`Cache`] trait, which allows it to be used
/// generically when some sort of cache is required.
///
/// [`Cache`]: trait.Cache.html
///
/// # Usage:
///
/// ```rust
/// use wasmer_runtime::cache::{Cache, FileSystemCache, WasmHash};
///
/// # use wasmer_runtime::{Module, error::CacheError};
/// fn store_module(module: Module) -> Result<Module, CacheError> {
///     // Create a new file system cache.
///     // This is unsafe because we can't ensure that the artifact wasn't
///     // corrupted or tampered with.
///     let mut fs_cache = unsafe { FileSystemCache::new("some/directory/goes/here")? };
///     // Compute a key for a given WebAssembly binary
///     let key = WasmHash::generate(&[]);
///     // Store a module into the cache given a key
///     fs_cache.store(key, module.clone())?;
///     Ok(module)
/// }
/// ```
pub struct FileSystemCache {
    path: PathBuf,
}

impl FileSystemCache {
    /// Construct a new `FileSystemCache` around the specified directory.
    /// The contents of the cache are stored in sub-versioned directories.
    ///
    /// # Note:
    /// This method is unsafe because there's no way to ensure the artifacts
    /// stored in this cache haven't been corrupted or tampered with.
    pub unsafe fn new<P: Into<PathBuf>>(path: P) -> io::Result<Self> {
        let path: PathBuf = path.into();
        if path.exists() {
            let metadata = path.metadata()?;
            if metadata.is_dir() {
                if !metadata.permissions().readonly() {
                    Ok(Self { path })
                } else {
                    // This directory is readonly.
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!("the supplied path is readonly: {}", path.display()),
                    ))
                }
            } else {
                // This path points to a file.
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "the supplied path already points to a file: {}",
                        path.display()
                    ),
                ))
            }
        } else {
            // Create the directory and any parent directories if they don't yet exist.
            create_dir_all(&path)?;
            Ok(Self { path })
        }
    }
}

impl Cache for FileSystemCache {
    type LoadError = CacheError;
    type StoreError = CacheError;

    fn load(&self, key: WasmHash) -> Result<Module, CacheError> {
        self.load_with_backend(key, Backend::default())
    }

    fn load_with_backend(&self, key: WasmHash, backend: Backend) -> Result<Module, CacheError> {
        let filename = key.encode();
        let mut new_path_buf = self.path.clone();
        new_path_buf.push(backend.to_string());
        new_path_buf.push(filename);
        let file = File::open(new_path_buf)?;
        let mmap = unsafe { Mmap::map(&file)? };

        let serialized_cache = Artifact::deserialize(&mmap[..])?;
        unsafe {
            wasmer_runtime_core_fl::load_cache_with(
                serialized_cache,
                crate::compiler_for_backend(backend)
                    .ok_or_else(|| CacheError::UnsupportedBackend(backend.to_string().to_owned()))?
                    .as_ref(),
            )
        }
    }

    fn store(&mut self, key: WasmHash, module: Module) -> Result<(), CacheError> {
        let filename = key.encode();
        let backend_str = module.info().backend.to_string();
        let mut new_path_buf = self.path.clone();
        new_path_buf.push(backend_str);

        let serialized_cache = module.cache()?;
        let buffer = serialized_cache.serialize()?;

        std::fs::create_dir_all(&new_path_buf)?;
        new_path_buf.push(filename);
        let mut file = File::create(new_path_buf)?;
        file.write_all(&buffer)?;

        Ok(())
    }
}
