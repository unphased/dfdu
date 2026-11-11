use super::*;

#[test]
fn verdict_wire_names() {
    for (verdict, name) in [
        (Verdict::HashMatch, "hash_match"),
        (Verdict::HashMismatch, "hash_mismatch"),
        (Verdict::Pending, "pending"),
        (Verdict::NameOnly, "name_only"),
        (Verdict::None, "none"),
    ] {
        let json = format!("\"{name}\"");
        assert_eq!(serde_json::to_string(&verdict).unwrap(), json);
        assert_eq!(serde_json::from_str::<Verdict>(&json).unwrap(), verdict);
    }
}

#[test]
fn sample_hash_samples_edges_and_length() {
    let dir = std::env::temp_dir().join(format!("dfdu-peer-sample-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("sample");
    for bytes in [b"".as_slice(), b"hello".as_slice()] {
        fs::write(&path, bytes).unwrap();
        let hash = sample_hash(&path).unwrap();
        assert_eq!(hash.len(), 32);
        assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(sample_hash(&path).unwrap(), hash);
    }
    fs::write(&path, b"hello").unwrap();
    let short = sample_hash(&path).unwrap();
    fs::write(&path, b"jello").unwrap();
    assert_ne!(sample_hash(&path).unwrap(), short);
    let mut bytes = vec![b'a'; 150 * 1024];
    fs::write(&path, &bytes).unwrap();
    let original = sample_hash(&path).unwrap();
    bytes[75 * 1024] = b'b';
    fs::write(&path, &bytes).unwrap();
    assert_eq!(sample_hash(&path).unwrap(), original);
    bytes[0] = b'b';
    fs::write(&path, &bytes).unwrap();
    assert_ne!(sample_hash(&path).unwrap(), original);
    bytes[0] = b'a';
    *bytes.last_mut().unwrap() = b'b';
    fs::write(&path, &bytes).unwrap();
    assert_ne!(sample_hash(&path).unwrap(), original);
    *bytes.last_mut().unwrap() = b'a';
    bytes.push(b'a');
    fs::write(&path, &bytes).unwrap();
    assert_ne!(sample_hash(&path).unwrap(), original);
    fs::remove_dir_all(dir).unwrap();
    assert!(sample_hash(&path).is_err());
}
