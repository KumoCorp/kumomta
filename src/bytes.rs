///! Various utility functions to operate on bytes

pub(crate) fn get_all_after<'a>(bytes: &'a [u8], end: &[u8]) -> &'a [u8] {
    if let Some(mut end_index) = find(bytes, end) {
        end_index += end.len();
        &bytes[end_index..]
    } else {
        &[]
    }
}

/// Find the offset of specific bytes in bytes
pub(crate) fn find(bytes: &[u8], search: &[u8]) -> Option<usize> {
    bytes
        .windows(search.len())
        .position(|window| window == search)
}

pub(crate) fn replace(bytes: &mut [u8], from: char, to: char) {
    for byte in bytes.iter_mut() {
        if *byte == from as u8 {
            *byte = to as u8;
        }
    }
}

pub(crate) fn replace_slice(source: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut result = source.to_vec();
    let from_len = from.len();
    let to_len = to.len();

    let mut i = 0;
    while i + from_len <= result.len() {
        if result[i..].starts_with(from) {
            result.splice(i..i + from_len, to.iter().cloned());
            i += to_len;
        } else {
            i += 1;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_find() {
        assert_eq!(find(&[97, 98, 99], &[1]), None);
        assert_eq!(find(&[97, 98, 99], &[97]), Some(0));
        assert_eq!(find(&[97, 98, 99], &[97, 98]), Some(0));
    }

    #[test]
    fn it_replace_slice() {
        let source = "aba".as_bytes();
        assert_eq!(replace_slice(source, &[97], &[99]), "cbc".as_bytes());
        assert_eq!(replace_slice(source, &[97, 98], &[]), "a".as_bytes());
    }
}
