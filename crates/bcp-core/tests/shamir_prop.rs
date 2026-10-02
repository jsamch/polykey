use bcp_core::shamir::{combine, split, OsRng, Share, SECRET_LEN};
use proptest::prelude::*;

fn pick(shares: &[Share], order: &[usize], count: usize) -> Vec<Share> {
    order[..count].iter().map(|&i| shares[i].clone()).collect()
}

fn params() -> impl Strategy<Value = (u8, u8)> {
    (2u8..=20).prop_flat_map(|n| (2u8..=n, Just(n)))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn round_trip_and_threshold(
        secret in prop::array::uniform32(any::<u8>()),
        (k, n) in params(),
        seed in any::<u64>(),
    ) {
        let shares = split(&secret, k, n, &mut OsRng).unwrap();
        prop_assert_eq!(shares.len(), n as usize);

        // Deterministic shuffle of share indexes derived from the seed.
        let mut order: Vec<usize> = (0..n as usize).collect();
        let mut state = seed | 1;
        for i in (1..order.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            order.swap(i, (state % (i as u64 + 1)) as usize);
        }

        let got = combine(&pick(&shares, &order, k as usize)).unwrap();
        prop_assert_eq!(&got[..], &secret[..]);
        let all = combine(&pick(&shares, &order, n as usize)).unwrap();
        prop_assert_eq!(&all[..], &secret[..]);

        if k > 2 {
            let short = combine(&pick(&shares, &order, k as usize - 1)).unwrap();
            prop_assert_ne!(&short[..], &secret[..]);
        }
    }
}

#[test]
fn secret_len_is_32() {
    assert_eq!(SECRET_LEN, 32);
}
