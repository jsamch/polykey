use bcp_core::codec::*;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

fn hex_field(len: usize) -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(b"0123456789ABCDEF".to_vec()), len)
        .prop_map(|v| v.into_iter().map(char::from).collect())
}

fn share_params() -> impl Strategy<Value = (u8, u8, u8)> {
    (2u8..=255).prop_flat_map(|n| (1u8..=n, 2u8..=n, Just(n)))
}

/// Substitutes the character at `idx` (chars) with `c`.
fn subst(s: &str, idx: usize, c: char) -> String {
    s.chars()
        .enumerate()
        .map(|(i, o)| if i == idx { c } else { o })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn share_round_trips(
        (x, k, n) in share_params(),
        sid in hex_field(8),
        data in prop::array::uniform32(any::<u8>()),
        ver in prop::option::of(hex_field(3)),
    ) {
        let enc = encode_share(x, k, n, &sid, &data, ver.as_deref());
        let forms = [
            enc.clone(),
            qr_payload(&enc),
            enc.to_lowercase(),
            qr_payload(&enc).to_lowercase(),
        ];
        for f in &forms {
            let p = parse_share(f).unwrap();
            prop_assert_eq!((p.x, p.k, p.n), (x, k, n));
            prop_assert_eq!(&p.set_id, &sid);
            prop_assert_eq!(&p.data[..], &data[..]);
            prop_assert_eq!(&p.ver, &ver);
            prop_assert_eq!(p.tag, if ver.is_some() { Tag::Bcp2 } else { Tag::Bcp1 });
            prop_assert!(!is_master(f));
        }
    }

    #[test]
    fn master_round_trips(
        data in prop::array::uniform32(any::<u8>()),
        sid in hex_field(8),
        ver in prop::option::of(hex_field(3)),
        use_real_sid in any::<bool>(),
    ) {
        // BCPK1 must carry the key's own set ID.
        let sid = if ver.is_none() && use_real_sid { set_id(&data) } else { sid };
        let enc = encode_master(&sid, &data, ver.as_deref());
        for f in [enc.clone(), qr_payload(&enc), enc.to_lowercase(), qr_payload(&enc).to_lowercase()] {
            match parse_master(&f) {
                Ok(p) => {
                    prop_assert_eq!(&p.set_id, &sid);
                    prop_assert_eq!(&p.data[..], &data[..]);
                    prop_assert_eq!(&p.ver, &ver);
                    prop_assert!(is_master(&f));
                }
                Err(e) => {
                    // Only an unlocked plate with a made-up set ID may fail, on that.
                    prop_assert!(ver.is_none() && sid != set_id(&data));
                    prop_assert_eq!(e, ParseError::SetIdMismatch);
                }
            }
        }
    }

}

// A 16-bit CHECK can collide by chance (about 1 in 65536 per substitution), so this property
// runs on a fixed seed to stay deterministic.
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 512,
        rng_seed: RngSeed::Fixed(0x2_2C0DEC),
        ..ProptestConfig::default()
    })]

    #[test]
    fn single_substitution_is_rejected_or_identical(
        (x, k, n) in share_params(),
        sid in hex_field(8),
        data in prop::array::uniform32(any::<u8>()),
        ver in prop::option::of(hex_field(3)),
        pos in any::<prop::sample::Index>(),
        c in prop::sample::select(
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789:-= ".chars().collect::<Vec<_>>()),
    ) {
        let enc = encode_share(x, k, n, &sid, &data, ver.as_deref());
        let orig = parse_share(&enc).unwrap();
        let idx = pos.index(enc.chars().count());
        let bad = subst(&enc, idx, c);
        if let Ok(p) = parse_share(&bad) {
            prop_assert_eq!(p, orig);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn parse_never_panics_arbitrary(text in ".*") {
        let _ = canonical(&text);
        let _ = is_master(&text);
        let _ = parse_share(&text);
        let _ = parse_master(&text);
    }

    #[test]
    fn parse_never_panics_alphabet(text in "[A-Z0-9a-z:\\- \t=+_]{0,120}") {
        let _ = parse_share(&text);
        let _ = parse_master(&text);
    }

    #[test]
    fn parse_never_panics_near_valid(
        tag in prop::sample::select(vec!["BCP1", "BCP2", "BCPK1", "BCPK2"]),
        fields in prop::collection::vec(
            prop::string::string_regex("[A-Z2-7=0-9]{0,60}").unwrap(), 0..9),
        sep in prop::sample::select(vec![":", " ", "-", ": "]),
    ) {
        let text = format!("{tag}{sep}{}", fields.join(sep));
        let _ = parse_share(&text);
        let _ = parse_master(&text);
    }
}
