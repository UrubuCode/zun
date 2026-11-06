use super::*;
use std::path::PathBuf;

use zun_metadata::{TableKind, Token};
use zun_module::Module;

use crate::bytes::write_name;


    #[test]
    fn content_cache_deduplicates_identical_module_bytes() {
        let mut cache = ModuleCache::default();
        let first = cache.insert(b"module");
        let second = cache.insert(b"module");
        assert_eq!(first, second);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(first), Some(&b"module"[..]));
    }

    #[test]
    fn content_id_does_not_depend_on_module_name() {
        assert_eq!(content_id(b"same"), content_id(b"same"));
        assert_ne!(content_id(b"same"), content_id(b"different"));
    }

    #[test]
    fn filesystem_cache_roundtrips_and_rejects_corruption() {
        let root = std::env::temp_dir().join(format!("zun-content-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut cache = ModuleCache::default();
        let id = cache.store(&root, b"module bytes").unwrap();
        assert_eq!(ModuleCache::load(&root, id).unwrap(), b"module bytes");
        std::fs::write(root.join(format!("{}.zm", id.hex())), b"tampered").unwrap();
        assert!(matches!(
            ModuleCache::load(&root, id),
            Err(CacheError::Integrity { .. })
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_store_repairs_a_corrupted_existing_content_address() {
        let root = std::env::temp_dir().join(format!("zun-content-cache-repair-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut cache = ModuleCache::default();
        let id = cache.store(&root, b"module bytes").unwrap();
        std::fs::write(root.join(format!("{}.zm", id.hex())), b"tampered").unwrap();
        assert_eq!(cache.store(&root, b"module bytes").unwrap(), id);
        assert_eq!(ModuleCache::load(&root, id).unwrap(), b"module bytes");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lockfile_roundtrips_content_identities_and_rejects_duplicates() {
        let mut lock = ModuleLock::new();
        let root = std::env::temp_dir().join(format!("zun-lock-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut cache = ModuleCache::default();
        let std_id = cache.store(&root, b"std").unwrap();
        let app_id = cache.store(&root, b"app").unwrap();
        lock.push("zun:std", std_id).unwrap();
        lock.push("app", app_id).unwrap();
        lock.grant_capabilities("app", ["net".to_owned(), "fs".to_owned(), "net".to_owned()])
            .unwrap();
        assert!(lock
            .grant_capabilities("zun:std", ["camera".to_owned()])
            .is_err());
        assert!(lock.push("app", content_id(b"other")).is_err());
        let decoded = ModuleLock::read(&lock.write()).unwrap();
        assert_eq!(decoded, lock);
        decoded.verify_cache(&root).unwrap();
        let mut metadata_lock = ModuleLock::new();
        metadata_lock.push("app", app_id).unwrap();
        metadata_lock
            .push("app.d.ts", content_id(b"declare const x: int;"))
            .unwrap();
        metadata_lock.verify_cache(&root).unwrap();
        assert_eq!(
            decoded.capabilities,
            vec![("app".to_owned(), vec!["fs".to_owned(), "net".to_owned()])]
        );
        assert!(decoded.require_capability("app", "fs").is_ok());
        assert!(decoded.require_capability("app", "camera").is_err());
        let path = std::env::temp_dir().join(format!("zun-lock-{}.zlock", std::process::id()));
        lock.write_file(&path).unwrap();
        assert_eq!(ModuleLock::read_file(&path).unwrap(), lock);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn signed_lockfile_roundtrips_and_rejects_tampering() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(b"app")).unwrap();
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let signature = signing.sign(&lock.write()).to_bytes();
        let signed =
            SignedModuleLock::new(lock, signing.verifying_key().to_bytes(), signature).unwrap();
        let bytes = signed.write();
        assert_eq!(SignedModuleLock::read(&bytes).unwrap(), signed);
        assert!(signed
            .verify_against(&signing.verifying_key().to_bytes())
            .is_ok());
        assert!(signed.verify_against(&[99u8; 32]).is_err());
        let mut tampered = bytes;
        *tampered.last_mut().unwrap() ^= 1;
        assert!(SignedModuleLock::read(&tampered).is_err());
    }

    #[test]
    fn signed_lockfile_valid_anchored_key_passes() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(b"app")).unwrap();
        let signing = SigningKey::from_bytes(&[21u8; 32]);
        let lock_bytes = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            signing.verifying_key().to_bytes(),
            signing.sign(&lock_bytes).to_bytes(),
        )
        .unwrap();
        let trusted = TrustedKeySet::new([signing.verifying_key().to_bytes()]).unwrap();
        assert!(signed.verify(&trusted).is_ok());
    }

    #[test]
    fn trusted_signed_lock_caches_only_after_anchor_verification() {
        use ed25519_dalek::{Signer, SigningKey};
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, _name: &str) -> Result<Vec<u8>, LoadError> {
                Ok(self.0.clone())
            }
        }
        let root = std::env::temp_dir().join(format!("zun-loader-signed-lock-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bytes = Module::new().write();
        let id = content_id(&bytes);
        let mut lock = ModuleLock::new();
        lock.push("signed-lock", id).unwrap();
        let signing = SigningKey::from_bytes(&[71u8; 32]);
        let payload = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            signing.verifying_key().to_bytes(),
            signing.sign(&payload).to_bytes(),
        )
        .unwrap();
        let trusted = TrustedKeySet::new([signing.verifying_key().to_bytes()]).unwrap();
        let mut loader = ModuleLoader::new();
        loader
            .load_signed_lock_with_keys_cached(&root, &signed, &trusted, &Registry(bytes.clone()))
            .unwrap();
        assert!(loader.get("signed-lock").is_some());
        assert_eq!(ModuleCache::load(&root, id).unwrap(), bytes);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn signed_lockfile_invalid_self_declared_key_is_rejected() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(b"app")).unwrap();
        let attacker = SigningKey::from_bytes(&[22u8; 32]);
        let lock_bytes = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            attacker.verifying_key().to_bytes(),
            attacker.sign(&lock_bytes).to_bytes(),
        )
        .unwrap();
        let trusted = TrustedKeySet::new([[23u8; 32]]).unwrap();
        let decoded = SignedModuleLock::read(&signed.write()).unwrap();
        assert!(decoded.verify(&trusted).is_err());
    }

    #[test]
    fn trusted_key_set_valid_rotation_and_invalid_empty_revocation() {
        let old = [41u8; 32];
        let next = [42u8; 32];
        let trusted = TrustedKeySet::new([old]).unwrap();
        let rotated = trusted.rotate([next], [old]).unwrap();
        assert!(!rotated.contains(&old));
        assert!(rotated.contains(&next));
        assert!(rotated.revoke(next).is_err());
    }

    #[test]
    fn zmlib_roundtrips_signed_modules_and_rejects_identity_mismatch() {
        use ed25519_dalek::{Signer, SigningKey};
        let payload = b"serialized zm".to_vec();
        let declaration = b"declare function main(): int;".to_vec();
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(&payload)).unwrap();
        lock.push("app.d.ts", content_id(&declaration)).unwrap();
        let signing = SigningKey::from_bytes(&[9u8; 32]);
        let lock_bytes = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            signing.verifying_key().to_bytes(),
            signing.sign(&lock_bytes).to_bytes(),
        )
        .unwrap();
        let package = Zmlib::new(signed, vec![("app".into(), payload.clone())])
            .unwrap()
            .with_type_declarations(vec![("app".into(), declaration.clone())])
            .unwrap();
        assert!(package
            .verify_against(&signing.verifying_key().to_bytes())
            .is_ok());
        assert!(package.verify_against(&[99u8; 32]).is_err());
        assert_eq!(Zmlib::read(&package.write()).unwrap(), package);
        assert!(
            Zmlib::new(package.lock.clone(), vec![("app".into(), payload.clone())])
                .unwrap()
                .with_type_declarations(vec![("missing".into(), b"declare const x: int;".to_vec())])
                .is_err()
        );
        assert!(
            Zmlib::new(package.lock.clone(), vec![("app".into(), payload.clone())])
                .unwrap()
                .with_type_declarations(vec![
                    ("app".into(), b"declare function main(): int;".to_vec()),
                    ("app".into(), b"declare const x: int;".to_vec()),
                ])
                .is_err()
        );
        assert!(
            Zmlib::new(package.lock.clone(), vec![("app".into(), payload.clone())])
                .unwrap()
                .with_type_declarations(vec![("app".into(), b"tampered declarations".to_vec())])
                .is_err()
        );
        let path = std::env::temp_dir().join(format!("zun-package-{}.zmlib", std::process::id()));
        package.write_file(&path).unwrap();
        let loaded = Zmlib::read_file(&path).unwrap();
        let mut loader = ModuleLoader::new();
        assert!(loader.load_zmlib(&loaded).is_err());
        std::fs::remove_file(path).unwrap();
        assert!(Zmlib::new(package.lock, vec![("app".into(), b"tampered".to_vec())]).is_err());
    }

    #[test]
    fn zmlib_materializes_native_object_content_addressed_and_reuses_it() {
        use ed25519_dalek::{Signer, SigningKey};
        let payload = b"serialized zm".to_vec();
        let native = b"fake-native-bytes".to_vec();
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(&payload)).unwrap();
        lock.push("t-host.nativeobj", content_id(&native)).unwrap();
        let signing = SigningKey::from_bytes(&[9u8; 32]);
        let lock_bytes = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            signing.verifying_key().to_bytes(),
            signing.sign(&lock_bytes).to_bytes(),
        )
        .unwrap();
        let package = Zmlib::new(signed.clone(), vec![("app".into(), payload.clone())])
            .unwrap()
            .with_native_objects(vec![("t-host".into(), native.clone())])
            .unwrap();
        let root = std::env::temp_dir().join(format!("zun-nat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(package.materialize_native_object(&root, "other").unwrap(), None);
        let path = package
            .materialize_native_object(&root, "t-host")
            .unwrap()
            .unwrap();
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(&content_id(&native).hex()));
        assert_eq!(std::fs::read(&path).unwrap(), native);
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();
        let again = package
            .materialize_native_object(&root, "t-host")
            .unwrap()
            .unwrap();
        assert_eq!(again, path);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), first);
        // A corrupted cache file is repaired.
        std::fs::write(&path, b"junk").unwrap();
        package.materialize_native_object(&root, "t-host").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), native);
        // Pure-ZIL package: no native step.
        let pure = Zmlib::new(signed, vec![("app".into(), payload)]).unwrap();
        assert_eq!(pure.materialize_native_object(&root, "t-host").unwrap(), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn zmlib_embeds_authenticated_native_objects_per_triple() {
        use ed25519_dalek::{Signer, SigningKey};
        let payload = b"serialized zm".to_vec();
        let win = b"MZ-native-windows".to_vec();
        let lin = b"ELF-native-linux".to_vec();
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(&payload)).unwrap();
        lock.push("x86_64-pc-windows-msvc.nativeobj", content_id(&win))
            .unwrap();
        lock.push("x86_64-unknown-linux-gnu.nativeobj", content_id(&lin))
            .unwrap();
        let signing = SigningKey::from_bytes(&[9u8; 32]);
        let lock_bytes = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            signing.verifying_key().to_bytes(),
            signing.sign(&lock_bytes).to_bytes(),
        )
        .unwrap();
        let objects = vec![
            ("x86_64-pc-windows-msvc".to_string(), win.clone()),
            ("x86_64-unknown-linux-gnu".to_string(), lin.clone()),
        ];
        let package = Zmlib::new(signed.clone(), vec![("app".into(), payload.clone())])
            .unwrap()
            .with_native_objects(objects.clone())
            .unwrap();
        let bytes = package.write();
        let read = Zmlib::read(&bytes).unwrap();
        assert_eq!(read.native_objects, objects);
        assert_eq!(read, package);
        assert_eq!(
            read.native_object_for("x86_64-pc-windows-msvc"),
            Some(&win[..])
        );
        assert_eq!(read.native_object_for("aarch64-apple-darwin"), None);
        // Tampering a native payload byte fails validation.
        let mut tampered = bytes.clone();
        let last = tampered.len() - 5; // last native payload byte (before the u32 contract)
        tampered[last] ^= 0xff;
        assert!(Zmlib::read(&tampered).is_err());
        assert!(
            Zmlib::new(signed.clone(), vec![("app".into(), payload.clone())])
                .unwrap()
                .with_native_objects(vec![("x86_64-pc-windows-msvc".into(), b"bad".to_vec())])
                .is_err()
        );
        // Version-2 package (no native section) still reads.
        let v2 = Zmlib::new(signed, vec![("app".into(), payload)]).unwrap();
        let mut v2_bytes = v2.write();
        v2_bytes[5..7].copy_from_slice(&2u16.to_le_bytes());
        v2_bytes.truncate(v2_bytes.len() - 8); // drop native count + core contract
        let old = Zmlib::read(&v2_bytes).unwrap();
        assert!(old.native_objects.is_empty());
        assert_eq!(old.required_core_version, 0);
    }

    #[test]
    fn zmlib_core_contract_roundtrip_legacy_and_check() {
        use ed25519_dalek::{Signer, SigningKey};
        let payload = Module::new().write();
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(&payload)).unwrap();
        let key = SigningKey::from_bytes(&[33u8; 32]);
        let lock_bytes = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            key.verifying_key().to_bytes(),
            key.sign(&lock_bytes).to_bytes(),
        )
        .unwrap();
        let mut package = Zmlib::new(signed, vec![("app".into(), payload)]).unwrap();
        assert_eq!(package.required_core_version, zun_module::CORE_CONTRACT_VERSION);
        package.required_core_version = 7;
        let bytes = package.write();
        // Round-trip preserves the field.
        assert_eq!(Zmlib::read_unchecked(&bytes).unwrap().required_core_version, 7);
        // Mismatch is rejected with both versions named; equal is accepted.
        let err = Zmlib::read_for_engine(&bytes, 8).unwrap_err().to_string();
        assert!(err.contains("requires core contract v7") && err.contains("engine provides v8"));
        assert!(Zmlib::read_for_engine(&bytes, 7).is_ok());
        assert!(Zmlib::read(&bytes).is_err()); // running engine is not v7
        // v3 package (no field) reads as 0 = unspecified and is accepted.
        let mut v3 = bytes.clone();
        v3[5..7].copy_from_slice(&3u16.to_le_bytes());
        v3.truncate(v3.len() - 4);
        let old = Zmlib::read_for_engine(&v3, 99).unwrap();
        assert_eq!(old.required_core_version, 0);
        // The loader path checks too.
        let mut loader = ModuleLoader::new_with_trusted_keys(
            TrustedKeySet::new([key.verifying_key().to_bytes()]).unwrap(),
        );
        assert!(loader.load_zmlib(&package).is_err());
    }

    #[test]
    fn loader_registers_a_verified_cached_module() {
        let root = std::env::temp_dir().join(format!("zun-loader-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bytes = Module::new().write();
        let mut cache = ModuleCache::default();
        let id = cache.store(&root, &bytes).unwrap();
        let mut loader = ModuleLoader::new();
        loader.load_cached(&root, "cached", id).unwrap();
        assert!(loader.get("cached").is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trusted_loader_rejects_an_unanchored_package_key() {
        use ed25519_dalek::{Signer, SigningKey};
        let payload = Module::new().write();
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(&payload)).unwrap();
        let attacker = SigningKey::from_bytes(&[31u8; 32]);
        let lock_bytes = lock.write();
        let signed = SignedModuleLock::new(
            lock,
            attacker.verifying_key().to_bytes(),
            attacker.sign(&lock_bytes).to_bytes(),
        )
        .unwrap();
        let package = Zmlib::new(signed, vec![("app".into(), payload)]).unwrap();
        let trusted = TrustedKeySet::new([[32u8; 32]]).unwrap();
        let mut loader = ModuleLoader::new_with_trusted_keys(trusted);
        assert!(loader.load_zmlib(&package).is_err());
    }

    #[test]
    fn loader_verifies_fetched_content_before_registration() {
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, _name: &str) -> Result<Vec<u8>, LoadError> {
                Ok(self.0.clone())
            }
        }
        let bytes = Module::new().write();
        let registry = Registry(bytes.clone());
        let mut loader = ModuleLoader::new();
        loader
            .load_fetched("remote", content_id(&bytes), &registry)
            .unwrap();
        assert!(loader.get("remote").is_some());
        assert!(loader
            .load_fetched("bad", content_id(b"other"), &registry)
            .is_err());
        let mut lock = ModuleLock::new();
        lock.push("remote-a", content_id(&bytes)).unwrap();
        lock.push("remote-b", content_id(&bytes)).unwrap();
        let mut all = ModuleLoader::new();
        all.load_lock(&lock, &registry).unwrap();
        assert!(all.get("remote-a").is_some());
        assert!(all.get("remote-b").is_some());
    }

    #[test]
    fn loader_caches_verified_fetched_content_before_registration() {
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, _name: &str) -> Result<Vec<u8>, LoadError> {
                Ok(self.0.clone())
            }
        }
        let root = std::env::temp_dir().join(format!("zun-loader-fetch-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bytes = Module::new().write();
        let id = content_id(&bytes);
        let mut loader = ModuleLoader::new();
        loader
            .load_fetched_cached(&root, "remote", id, &Registry(bytes.clone()))
            .unwrap();
        assert!(loader.get("remote").is_some());
        assert_eq!(ModuleCache::load(&root, id).unwrap(), bytes);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bulk_cached_loads_preserve_lock_and_index_order() {
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, _name: &str) -> Result<Vec<u8>, LoadError> {
                Ok(self.0.clone())
            }
        }
        let root = std::env::temp_dir().join(format!("zun-loader-bulk-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bytes = Module::new().write();
        let id = content_id(&bytes);
        let registry = Registry(bytes);
        let mut lock = ModuleLock::new();
        lock.push("lock-a", id).unwrap();
        lock.push("lock-b", id).unwrap();
        let mut loader = ModuleLoader::new();
        loader.load_lock_cached(&root, &lock, &registry).unwrap();
        assert!(loader.get("lock-a").is_some());
        assert!(loader.get("lock-b").is_some());

        let mut index = ModuleIndex::new();
        index.push("index-a", id).unwrap();
        let mut indexed = ModuleLoader::new();
        indexed.load_index_cached(&root, &index, &registry).unwrap();
        assert!(indexed.get("index-a").is_some());
        assert!(ModuleCache::load(&root, id).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_index_roundtrips_and_rejects_duplicate_names() {
        let mut index = ModuleIndex::new();
        let id = content_id(b"module");
        index.push("@scope/module", id).unwrap();
        assert_eq!(index.resolve("@scope/module"), Some(id));
        assert_eq!(index.resolve("missing"), None);
        assert!(index.push("@scope/module", id).is_err());
        assert_eq!(ModuleIndex::read(&index.write()).unwrap(), index);
        let mut trailing = index.write();
        trailing.push(0);
        assert!(ModuleIndex::read(&trailing).is_err());
    }

    #[test]
    fn signed_registry_index_roundtrips_and_rejects_tampering() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut index = ModuleIndex::new();
        index.push("app", content_id(b"app")).unwrap();
        let signing = SigningKey::from_bytes(&[13u8; 32]);
        let signature = signing.sign(&index.write()).to_bytes();
        let signed =
            SignedModuleIndex::new(index, signing.verifying_key().to_bytes(), signature).unwrap();
        let bytes = signed.write();
        assert_eq!(SignedModuleIndex::read(&bytes).unwrap(), signed);
        assert!(signed
            .verify_against(&signing.verifying_key().to_bytes())
            .is_ok());
        assert!(signed.verify_against(&[99u8; 32]).is_err());
        let mut tampered = bytes;
        *tampered.last_mut().unwrap() ^= 1;
        assert!(SignedModuleIndex::read(&tampered).is_err());
    }

    #[test]
    fn signed_registry_index_valid_anchored_key_passes() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut index = ModuleIndex::new();
        index.push("app", content_id(b"app")).unwrap();
        let signing = SigningKey::from_bytes(&[51u8; 32]);
        let payload = index.write();
        let signed = SignedModuleIndex::new(
            index,
            signing.verifying_key().to_bytes(),
            signing.sign(&payload).to_bytes(),
        )
        .unwrap();
        let trusted = TrustedKeySet::new([signing.verifying_key().to_bytes()]).unwrap();
        assert!(signed.verify_with_keys(&trusted).is_ok());
    }

    #[test]
    fn signed_registry_index_rotates_trusted_keys_after_authentication() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut index = ModuleIndex::new();
        index.push("app", content_id(b"app")).unwrap();
        let signing = SigningKey::from_bytes(&[72u8; 32]);
        let next = [73u8; 32];
        let key_additions = vec![next];
        let key_revocations = vec![signing.verifying_key().to_bytes()];
        let signed = SignedModuleIndex::new_with_rotation(
            index.clone(),
            signing.verifying_key().to_bytes(),
            key_additions.clone(),
            key_revocations.clone(),
            signing
                .sign(&SignedModuleIndex::signing_bytes(
                    &index,
                    &key_additions,
                    &key_revocations,
                ))
                .to_bytes(),
        )
        .unwrap();
        let roundtrip = SignedModuleIndex::read(&signed.write()).unwrap();
        let trusted = TrustedKeySet::new([signing.verifying_key().to_bytes()]).unwrap();
        let rotated = roundtrip.rotated_trusted_keys(&trusted).unwrap();
        assert!(!rotated.contains(&signing.verifying_key().to_bytes()));
        assert!(rotated.contains(&next));
        let mut loader = ModuleLoader::new_with_trusted_keys(trusted);
        loader.apply_signed_index_rotation(&roundtrip).unwrap();
        let duplicate = vec![next, next];
        let duplicate_signature = signing
            .sign(&SignedModuleIndex::signing_bytes(&roundtrip.index, &duplicate, &[]))
            .to_bytes();
        assert!(SignedModuleIndex::new_with_rotation(
            roundtrip.index.clone(),
            signing.verifying_key().to_bytes(),
            duplicate,
            [],
            duplicate_signature,
        )
        .is_err());
    }

    #[test]
    fn trusted_signed_index_caches_only_after_anchor_verification() {
        use ed25519_dalek::{Signer, SigningKey};
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, _name: &str) -> Result<Vec<u8>, LoadError> {
                Ok(self.0.clone())
            }
        }
        let root = std::env::temp_dir().join(format!("zun-loader-signed-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bytes = Module::new().write();
        let id = content_id(&bytes);
        let mut index = ModuleIndex::new();
        index.push("signed", id).unwrap();
        let signing = SigningKey::from_bytes(&[61u8; 32]);
        let payload = index.write();
        let signed = SignedModuleIndex::new(
            index,
            signing.verifying_key().to_bytes(),
            signing.sign(&payload).to_bytes(),
        )
        .unwrap();
        let trusted = TrustedKeySet::new([signing.verifying_key().to_bytes()]).unwrap();
        let mut loader = ModuleLoader::new();
        loader
            .load_signed_index_with_keys_cached(&root, &signed, &trusted, &Registry(bytes.clone()))
            .unwrap();
        assert!(loader.get("signed").is_some());
        assert_eq!(ModuleCache::load(&root, id).unwrap(), bytes);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn signed_registry_index_invalid_self_declared_key_is_rejected() {
        use ed25519_dalek::{Signer, SigningKey};
        let mut index = ModuleIndex::new();
        index.push("app", content_id(b"app")).unwrap();
        let attacker = SigningKey::from_bytes(&[52u8; 32]);
        let payload = index.write();
        let signed = SignedModuleIndex::new(
            index,
            attacker.verifying_key().to_bytes(),
            attacker.sign(&payload).to_bytes(),
        )
        .unwrap();
        let trusted = TrustedKeySet::new([[53u8; 32]]).unwrap();
        assert!(signed.verify_with_keys(&trusted).is_err());
    }

    #[test]
    fn loader_consumes_registry_index_with_content_verification() {
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, _name: &str) -> Result<Vec<u8>, LoadError> {
                Ok(self.0.clone())
            }
        }
        let bytes = Module::new().write();
        let id = content_id(&bytes);
        let mut index = ModuleIndex::new();
        index.push("indexed", id).unwrap();
        let mut loader = ModuleLoader::new();
        loader.load_index(&index, &Registry(bytes)).unwrap();
        assert!(loader.get("indexed").is_some());

        let mut bad = ModuleIndex::new();
        bad.push("indexed", content_id(b"different")).unwrap();
        assert!(ModuleLoader::new()
            .load_index(&bad, &Registry(Module::new().write()))
            .is_err());
    }

    #[test]
    fn loader_requires_the_pinned_key_before_signed_index_fetches() {
        use ed25519_dalek::{Signer, SigningKey};
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, _name: &str) -> Result<Vec<u8>, LoadError> {
                Ok(self.0.clone())
            }
        }
        let bytes = Module::new().write();
        let mut index = ModuleIndex::new();
        index.push("indexed", content_id(&bytes)).unwrap();
        let signing = SigningKey::from_bytes(&[17u8; 32]);
        let index_bytes = index.write();
        let signed = SignedModuleIndex::new(
            index,
            signing.verifying_key().to_bytes(),
            signing.sign(&index_bytes).to_bytes(),
        )
        .unwrap();
        let mut loader = ModuleLoader::new();
        assert!(loader
            .load_signed_index(&signed, &[99u8; 32], &Registry(bytes.clone()))
            .is_err());
        loader
            .load_signed_index(&signed, &signing.verifying_key().to_bytes(), &Registry(bytes))
            .unwrap();
        assert!(loader.get("indexed").is_some());
        let mut trusted_loader = ModuleLoader::new_with_trusted_keys(
            TrustedKeySet::new([signing.verifying_key().to_bytes()]).unwrap(),
        );
        trusted_loader
            .load_signed_index_trusted(&signed, &Registry(Module::new().write()))
            .unwrap();
        assert!(trusted_loader.get("indexed").is_some());
    }

    #[test]
    fn loader_skips_metadata_entries_in_signed_lock_fetches() {
        struct Registry(Vec<u8>);
        impl ModuleFetcher for Registry {
            fn fetch(&self, name: &str) -> Result<Vec<u8>, LoadError> {
                if name.ends_with(".d.ts") {
                    return Err(LoadError::InvalidManifest(format!(
                        "metadata fetch attempted: {name}"
                    )));
                }
                Ok(self.0.clone())
            }
        }
        let bytes = Module::new().write();
        let mut lock = ModuleLock::new();
        lock.push("app", content_id(&bytes)).unwrap();
        lock.push("app.d.ts", content_id(b"declare const x: int;"))
            .unwrap();
        let mut loader = ModuleLoader::new();
        loader.load_lock(&lock, &Registry(bytes)).unwrap();
        assert!(loader.get("app").is_some());
        assert!(loader.get("app.d.ts").is_none());
    }

    #[test]
    fn directory_fetcher_loads_indexed_modules_and_rejects_escape_paths() {
        let root = std::env::temp_dir().join(format!("zun-registry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("scope")).unwrap();
        let bytes = Module::new().write();
        std::fs::write(root.join("scope").join("module.zm"), &bytes).unwrap();
        let mut index = ModuleIndex::new();
        index.push("scope/module", content_id(&bytes)).unwrap();
        let fetcher = DirectoryFetcher::new(&root);
        let mut loader = ModuleLoader::new();
        loader.load_index(&index, &fetcher).unwrap();
        assert!(loader.get("scope/module").is_some());
        assert!(fetcher.fetch("../outside").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn http_fetcher_validates_registry_urls_and_module_paths() {
        assert!(HttpFetcher::new("file:///tmp/registry").is_err());
        assert!(HttpFetcher::new("https://registry.example.test/root?tenant=evil").is_err());
        assert!(HttpFetcher::new("https://registry.example.test/root#fragment").is_err());
        assert!(HttpFetcher::new("https://registry.example.test/root\\mirror").is_err());
        let fetcher = HttpFetcher::new("https://registry.example.test/root").unwrap();
        assert!(fetcher.fetch("../escape").is_err());
        assert!(fetcher.fetch("scope\\escape").is_err());
        assert!(fetcher.fetch("scope/module?redirect=evil").is_err());
        assert!(fetcher.fetch("scope/module#fragment").is_err());
        assert!(fetcher.fetch("").is_err());
    }

    #[test]
    fn http_fetcher_loads_a_trusted_signed_index_and_module() {
        use ed25519_dalek::{Signer, SigningKey};
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let module = Module::new().write();
        let mut index = ModuleIndex::new();
        index.push("app", content_id(&module)).unwrap();
        let signing = SigningKey::from_bytes(&[71u8; 32]);
        let payload = index.write();
        let signed = SignedModuleIndex::new(
            index,
            signing.verifying_key().to_bytes(),
            signing.sign(&payload).to_bytes(),
        )
        .unwrap();
        let index_bytes = signed.write();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0u8; 1024];
                let size = stream.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..size]);
                let body = if request.starts_with("GET /index.zidx") {
                    &index_bytes
                } else {
                    &module
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(body).unwrap();
            }
        });

        let fetcher = HttpFetcher::new(format!("http://{address}")).unwrap();
        let signed = fetcher.fetch_signed_index().unwrap();
        let trusted = TrustedKeySet::new([signing.verifying_key().to_bytes()]).unwrap();
        let mut loader = ModuleLoader::new();
        loader
            .load_signed_index_with_keys(&signed, &trusted, &fetcher)
            .unwrap();
        assert!(loader.get("app").is_some());
        server.join().unwrap();
    }

    #[test]
    fn loads_and_preserves_scope_identity() {
        let module = Module::new();
        let scope = ModuleScope::from_bytes("demo", &module.write()).expect("load");
        assert_eq!(scope.name(), "demo");
    }

    #[test]
    fn indexes_module_exports_by_name() {
        let mut module = Module::new();
        let point = module.strings.intern("Point");
        let main = module.strings.intern("main");
        module.tables.add_type_def(zun_metadata::TypeDef {
            name: point,
            namespace: zun_metadata::StringIndex(0),
            base: Token::null(TableKind::TypeDef),
            field_start: 1,
            field_end: 1,
            method_start: 1,
            method_end: 1,
        });
        module.tables.add_method_def(zun_metadata::MethodDef {
            name: main,
            signature: zun_metadata::BlobIndex(0),
            body_offset: u32::MAX,
            is_static: false,
        });
        let scope = ModuleScope::from_bytes("demo", &module.write()).expect("load");
        assert!(matches!(scope.export("Point"), Some(Export::Type(_))));
        assert!(matches!(scope.export("main"), Some(Export::Method(_))));
        assert_eq!(scope.export("missing"), None);
    }

    #[test]
    fn registry_resolves_multiple_module_scopes() {
        let bytes = Module::new().write();
        let mut loader = ModuleLoader::new();
        loader.load_bytes("core", &bytes).expect("load core");
        loader.load_bytes("app", &bytes).expect("load app");
        assert_eq!(loader.len(), 2);
        assert_eq!(loader.get("app").unwrap().name(), "app");
    }

    #[test]
    fn resolves_an_export_from_a_loaded_module() {
        let mut module = Module::new();
        let name = module.strings.intern("main");
        module.tables.add_method_def(zun_metadata::MethodDef {
            name,
            signature: zun_metadata::BlobIndex(0),
            body_offset: u32::MAX,
            is_static: false,
        });
        let mut loader = ModuleLoader::new();
        loader
            .insert(ModuleScope::from_bytes("app", &module.write()).expect("load"))
            .expect("register");
        let (scope, export) = loader.resolve_export("app", "main").expect("export");
        assert_eq!(scope.name(), "app");
        assert!(matches!(export, Export::Method(_)));
        assert!(loader.resolve_export("app", "missing").is_err());
    }

    #[test]
    fn rejects_duplicate_export_names() {
        let mut module = Module::new();
        let name = module.strings.intern("main");
        for _ in 0..2 {
            module.tables.add_method_def(zun_metadata::MethodDef {
                name,
                signature: zun_metadata::BlobIndex(0),
                body_offset: u32::MAX,
                is_static: false,
            });
        }
        assert!(matches!(
            ModuleScope::from_bytes("app", &module.write()),
            Err(LoadError::DuplicateExport(export)) if export == "main"
        ));
    }

    #[test]
    fn resolves_top_level_member_ref_through_module_ref() {
        let mut target = Module::new();
        let export_name = target.strings.intern("answer");
        target.tables.add_method_def(zun_metadata::MethodDef {
            name: export_name,
            signature: zun_metadata::BlobIndex(0),
            body_offset: u32::MAX,
            is_static: false,
        });

        let mut requester = Module::new();
        let module_name = requester.strings.intern("core");
        let member_name = requester.strings.intern("answer");
        let module_ref = requester
            .tables
            .add_module_ref(zun_metadata::ModuleRef { name: module_name });
        let member_ref = requester.tables.add_member_ref(zun_metadata::MemberRef {
            parent: module_ref,
            name: member_name,
            signature: zun_metadata::BlobIndex(0),
        });

        let target = ModuleScope::from_bytes("core", &target.write()).expect("target");
        let requester = ModuleScope::from_bytes("app", &requester.write()).expect("requester");
        let mut loader = ModuleLoader::new();
        loader.insert(target).expect("target registration");
        loader.insert(requester).expect("requester registration");
        let resolved = loader
            .resolve_member_ref(loader.get("app").unwrap(), member_ref)
            .expect("member ref");
        assert_eq!(resolved.scope.name(), "core");
        assert_eq!(resolved.kind, MemberKind::Method);
        assert_eq!(resolved.token.rid(), 1);
        let (scope, token) = loader
            .resolve_method_ref(loader.get("app").unwrap(), member_ref)
            .expect("method ref");
        assert_eq!(scope.name(), "core");
        assert_eq!(token.rid(), 1);
    }

    #[test]
    fn registry_rejects_duplicate_identities() {
        let bytes = Module::new().write();
        let mut loader = ModuleLoader::new();
        loader.load_bytes("core", &bytes).expect("first load");
        assert!(matches!(
            loader.load_bytes("core", &bytes),
            Err(LoadError::DuplicateModule(name)) if name == "core"
        ));
    }

    #[test]
    fn manifest_validates_entry_and_dependencies() {
        let bytes = Module::new().write();
        let mut loader = ModuleLoader::new();
        loader.load_bytes("core", &bytes).expect("core");
        loader.load_bytes("app", &bytes).expect("app");
        let manifest = ModuleManifest::new("app", vec!["core".into()]);
        assert!(manifest.validate(&loader).is_ok());
        let missing = ModuleManifest::new("app", vec!["missing".into()]);
        assert!(missing.validate(&loader).is_err());
        assert_eq!(
            manifest.load_order(&loader).expect("order"),
            vec!["core", "app"]
        );
        assert_eq!(
            ModuleManifest::read(&manifest.write()).expect("roundtrip"),
            manifest
        );
    }

    #[test]
    fn manifest_roundtrips_optional_native_object() {
        let manifest = ModuleManifest::new("app", vec!["core".into()])
            .with_native_object("app.o")
            .with_metadata_object("app.meta.o")
            .with_runtime_library("runtime.lib")
            .with_prelude_fingerprint(0x1234_5678_9abc_def0);
        let decoded = ModuleManifest::read(&manifest.write()).expect("roundtrip");
        assert_eq!(decoded, manifest);
        assert_eq!(decoded.prelude_fingerprint, Some(0x1234_5678_9abc_def0));
        decoded
            .validate_prelude_fingerprint(0x1234_5678_9abc_def0)
            .expect("matching prelude should validate");
        assert!(decoded.validate_prelude_fingerprint(0x42).is_err());
        assert_eq!(
            decoded.native_object_path("dist/app.zman"),
            Some(PathBuf::from("dist/app.o"))
        );
        assert_eq!(
            decoded.metadata_object_path("dist/app.zman"),
            Some(PathBuf::from("dist/app.meta.o"))
        );
        assert_eq!(
            decoded.runtime_library_path("dist/app.zman"),
            Some(PathBuf::from("dist/runtime.lib"))
        );
        assert_eq!(
            ModuleManifest::new("app", Vec::<String>::new()).native_object_path("app.zman"),
            None
        );
    }

    #[test]
    fn manifest_reads_version_zero_without_native_object() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(ModuleManifest::MAGIC);
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        write_name(&mut bytes, "app");
        let decoded = ModuleManifest::read(&bytes).expect("legacy manifest");
        assert_eq!(decoded, ModuleManifest::new("app", Vec::<String>::new()));
    }

    #[test]
    fn manifest_rejects_duplicate_or_self_dependencies() {
        let bytes = Module::new().write();
        let mut loader = ModuleLoader::new();
        loader.load_bytes("app", &bytes).expect("app");
        loader.load_bytes("x", &bytes).expect("x");
        assert!(matches!(
            ModuleManifest::new("app", vec!["app".into()]).validate(&loader),
            Err(LoadError::InvalidManifest(_))
        ));
        assert!(matches!(
            ModuleManifest::new("app", vec!["x".into(), "x".into()]).validate(&loader),
            Err(LoadError::InvalidManifest(_))
        ));
    }

    #[test]
    fn cache_aware_manifest_loading_rejects_a_stale_prelude_before_io() {
        let manifest = ModuleManifest::new("app", Vec::<String>::new()).with_prelude_fingerprint(7);
        let mut loader = ModuleLoader::new();
        let error = loader
            .load_manifest_with_prelude(&manifest, 8)
            .expect_err("stale prelude must be rejected");
        assert!(
            matches!(error, LoadError::InvalidManifest(detail) if detail.contains("fingerprint"))
        );
    }

    #[test]
    fn manifest_rejects_truncated_or_trailing_bytes() {
        let manifest = ModuleManifest::new("app", vec!["core".into()]);
        let bytes = manifest.write();
        assert!(ModuleManifest::read(&bytes[..bytes.len() - 1]).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(ModuleManifest::read(&trailing).is_err());
    }

    #[test]
    fn manifest_file_roundtrip() {
        let manifest = ModuleManifest::new("app", vec!["core".into()]);
        let path = std::env::temp_dir().join(format!(
            "zun-manifest-{}-{}.zman",
            std::process::id(),
            manifest.dependencies.len()
        ));
        manifest.write_file(&path).expect("write manifest");
        let loaded = ModuleManifest::read_file(&path).expect("read manifest");
        std::fs::remove_file(&path).expect("remove manifest");
        assert_eq!(loaded, manifest);
    }

    #[test]
    fn loads_manifest_files_in_dependency_order() {
        let root = std::env::temp_dir().join(format!("zun-loader-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create temp directory");
        let dependency = root.join("core.zm");
        let entry = root.join("app.zm");
        std::fs::write(&dependency, Module::new().write()).expect("write dependency");
        std::fs::write(&entry, Module::new().write()).expect("write entry");
        let manifest = ModuleManifest::new(
            entry.to_string_lossy(),
            [dependency.to_string_lossy().into_owned()],
        );
        let mut loader = ModuleLoader::new();
        loader.load_manifest(&manifest).expect("load manifest");
        assert_eq!(loader.len(), 2);
        assert!(loader.get(&entry.to_string_lossy()).is_some());
        std::fs::remove_dir_all(root).expect("remove temp directory");
    }

    #[test]
    fn rejects_manifest_with_unresolved_member_reference() {
        let mut module = Module::new();
        let missing = module.strings.intern("missing");
        let member = module.strings.intern("answer");
        let module_ref = module
            .tables
            .add_module_ref(zun_metadata::ModuleRef { name: missing });
        module.tables.add_member_ref(zun_metadata::MemberRef {
            parent: module_ref,
            name: member,
            signature: zun_metadata::BlobIndex(0),
        });
        let scope = ModuleScope::from_bytes("app", &module.write()).expect("load app");
        let mut loader = ModuleLoader::new();
        loader.insert(scope).expect("register app");
        let manifest = ModuleManifest::new("app", Vec::<String>::new());
        assert!(matches!(
            loader.validate_member_refs(&manifest),
            Err(LoadError::UnresolvedModule(name)) if name == "missing"
        ));
    }
