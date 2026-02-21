fn gcd(mut n1: usize, mut n2: usize) -> usize {
    while n1 != n2 {
        if n1 > n2 {
            n1 -= n2;
        } else {
            n2 -= n1;
        }
    }

    n1
}

fn lcm(n1: usize, n2: usize) -> usize {
    n1 * n2 / gcd(n1, n2)
}

fn get_bit(data: &[u8], n: usize) -> u8 {
    let pos_byte = n / 8;
    let pos_bit = n % 8;

    let val_byte = data[pos_byte];

    (val_byte >> (8 - (pos_bit + 1))) & 0x0001
}

fn set_bit(data: &mut [u8], pos: usize, val: u8) {
    let pos_byte = pos / 8;
    let pos_bit = pos % 8;

    let old_byte = data[pos_byte];
    let new_byte = (val << (8 - (pos_bit + 1))) | old_byte;
    data[pos_byte] = new_byte;
}

fn rotate_right(data: &[u8], len: usize, n: usize, out: &mut [u8]) {
    for i in 0..len {
        let val = get_bit(data, i);
        set_bit(out, (i + n) % len, val);
    }
}

fn sum(n1: &[u8], n2: &[u8], len: usize) -> Vec<u8> {
    let num_of_bytes = (len - 1) / 8 + 1;
    let mut out = vec![0; num_of_bytes];
    let mut carry = 0;

    for i in (0..len).rev() {
        let n1b = get_bit(n1, i);
        let n2b = get_bit(n2, i);

        let sum = n1b + n2b + carry;

        match sum {
            0 | 1 => {
                set_bit(&mut out, i, sum);
                carry = 0;
            }
            2 => carry = 1,
            3 => {
                set_bit(&mut out, i, 1);
                carry = 1;
            }
            _ => {}
        }
    }

    if carry == 1 {
        let mut carry_array = vec![0; n1.len()];
        carry_array[n1.len() - 1] = 1;
        out = sum(&out, &carry_array, n1.len() * 8);
    }

    out
}

pub fn n_fold(data: &[u8], n: usize) -> Vec<u8> {
    let k = data.len() * 8;

    let lcm = lcm(n, k);

    let replicate = lcm / k;

    let mut sum_bytes = vec![0; lcm / 8];

    for i in 0..replicate {
        let rotation = 13 * i;

        let len = data.len() * 8;
        let num_of_bytes = (len - 1) / 8 + 1;

        rotate_right(
            data,
            len,
            rotation,
            &mut sum_bytes[(i * num_of_bytes)..((i + 1) * num_of_bytes)],
        );
    }

    let l = n / 8;
    let mut sum_v = vec![0; l];
    let mut nfold = vec![0; l];

    for m in 0..(lcm / n) {
        for o in 0..l {
            sum_v[o] = sum_bytes[o + (m * n / 8)];
        }

        nfold = sum(&nfold, &sum_v, nfold.len() * 8);
    }

    nfold
}

#[cfg(test)]
mod tests {
    use crate::crypto::nfold::{gcd, lcm};

    use super::n_fold;

    #[test]
    fn test_gcd() {
        assert_eq!(4, gcd(8, 12));
        assert_eq!(4, gcd(12, 8));

        assert_eq!(6, gcd(54, 24));
        assert_eq!(6, gcd(24, 54));
    }

    #[test]
    fn test_lcm() {
        assert_eq!(12, lcm(4, 6));
        assert_eq!(12, lcm(6, 4));
    }

    #[test]
    fn test_nfold() {
        assert_eq!(&[190_u8, 7, 38, 49, 39, 107, 25, 85], n_fold(b"012345", 64).as_slice());

        assert_eq!(
            &[120_u8, 160, 123, 108, 175, 133, 250],
            n_fold(b"password", 56).as_slice()
        );

        assert_eq!(
            &[
                89_u8, 228, 168, 202, 124, 3, 133, 195, 195, 123, 63, 109, 32, 0, 36, 124, 182, 230, 189, 91, 62
            ],
            n_fold(b"password", 168).as_slice()
        );

        assert_eq!(
            &[
                81_u8, 138, 84, 162, 21, 168, 69, 42, 81, 138, 84, 162, 21, 168, 69, 42, 81, 138, 84, 162, 21
            ],
            n_fold(b"Q", 168).as_slice()
        );

        assert_eq!(
            &[
                251_u8, 37, 213, 49, 174, 137, 116, 73, 159, 82, 253, 146, 234, 152, 87, 196, 186, 36, 207, 41, 126
            ],
            n_fold(b"ba", 168).as_slice()
        );
    }
}
