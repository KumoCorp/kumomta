pub fn has_lone_cr_or_lf(data: &[u8]) -> bool {
    for i in memchr::memchr2_iter(b'\r', b'\n', data) {
        match data[i] {
            b'\r' => {
                if data.get(i + 1).copied() != Some(b'\n') {
                    return true;
                }
            }
            b'\n' => {
                if i == 0 || data[i - 1] != b'\r' {
                    return true;
                }
            }
            _ => unreachable!(),
        }
    }
    false
}

pub fn normalize_crlf(data: &[u8]) -> Vec<u8> {
    let mut normalized = Vec::with_capacity(data.len());
    let mut last_idx = 0;

    for i in memchr::memchr2_iter(b'\r', b'\n', data) {
        match data[i] {
            b'\r' => {
                normalized.extend_from_slice(&data[last_idx..=i]);
                if data.get(i + 1).copied() != Some(b'\n') {
                    normalized.push(b'\n');
                }
            }
            b'\n' => {
                normalized.extend_from_slice(&data[last_idx..i]);
                let needs_cr = i == 0 || data[i - 1] != b'\r';
                if needs_cr {
                    normalized.push(b'\r');
                }
                normalized.push(b'\n');
            }
            _ => unreachable!(),
        }
        last_idx = i + 1;
    }

    normalized.extend_from_slice(&data[last_idx..]);
    normalized
}

pub fn normalize_crlf_in_place(data: &mut Vec<u8>) {
    let mut idx = 0;
    'find_again: while idx < data.len() {
        for i in memchr::memchr2_iter(b'\r', b'\n', &data[idx..]) {
            match data[idx + i] {
                b'\r' => {
                    if data.get(idx + i + 1).copied() != Some(b'\n') {
                        data.insert(idx + i + 1, b'\n');
                        idx = idx + i + 2;
                        continue 'find_again;
                    }
                }
                b'\n' => {
                    let needs_cr = idx + i == 0 || data[idx + i - 1] != b'\r';
                    if needs_cr {
                        data.insert(idx + i, b'\r');
                        idx = idx + i + 2;
                        continue 'find_again;
                    }
                }
                _ => unreachable!(),
            }
        }
        return;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn loner() {
        assert!(!has_lone_cr_or_lf(b""));
        assert!(!has_lone_cr_or_lf(b"hello"));
        assert!(!has_lone_cr_or_lf(b"hello\r\nthere"));
        assert!(!has_lone_cr_or_lf(b"hello\r\nthere\r\n"));
        assert!(!has_lone_cr_or_lf(b"\r\nhello\r\nthere\r\n"));
        assert!(has_lone_cr_or_lf(b"hello\n"));
        assert!(has_lone_cr_or_lf(b"hello\r"));
        assert!(has_lone_cr_or_lf(b"\nhello\r\nthere\r\n"));
        assert!(has_lone_cr_or_lf(b"\rhello\r\nthere\r\n"));
        assert!(has_lone_cr_or_lf(b"hello\nthere\r\n"));
        assert!(has_lone_cr_or_lf(b"hello\r\nthere\n"));
        assert!(has_lone_cr_or_lf(b"hello\r\r\r\nthere\n"));
    }

    #[test]
    fn fix_loner() {
        fn fix(s: &[u8], expect: &[u8]) {
            let mut data = s.to_vec();
            normalize_crlf_in_place(&mut data);
            assert_eq!(data, expect);

            assert_eq!(normalize_crlf(s), expect);
        }

        fix(b"\nhello\r\nthere\r\n", b"\r\nhello\r\nthere\r\n");
        fix(b"hello\r", b"hello\r\n");
        fix(b"hello\nthere\r\n", b"hello\r\nthere\r\n");
        fix(b"hello\r\nthere\n", b"hello\r\nthere\r\n");
        fix(b"hello\r\r\r\nthere\n", b"hello\r\n\r\n\r\nthere\r\n");
    }

    #[test]
    fn test_normalize_crlf() {
        assert_eq!(
            normalize_crlf(b"foo\r\nbar\nwoot\rdouble-r\r\rend"),
            b"foo\r\nbar\r\nwoot\r\ndouble-r\r\n\r\nend"
        );
    }
}
