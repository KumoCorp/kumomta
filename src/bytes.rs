///! Various utility functions to operate on bytes
pub(crate) use memchr::memmem::find;

pub(crate) fn replace(bytes: &mut [u8], from: u8, to: u8) {
    let mut previous = 0;
    while let Some(idx) = memchr::memchr(from, &bytes[previous..]) {
        bytes[previous + idx] = to;
        previous = idx + 1;
    }
}

pub(crate) fn replace_within_vec(result: &mut Vec<u8>, from: &[u8], to: &[u8]) {
    let from_len = from.len();
    let to_len = to.len();

    let mut i = 0;
    while let Some(idx) = find(&result[i..], from) {
        result.splice(idx + i..idx + i + from_len, to.iter().cloned());
        i += idx + to_len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace_slice(source: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
        let mut result = source.to_vec();
        replace_within_vec(&mut result, from, to);
        result
    }

    #[test]
    fn it_find() {
        assert_eq!(find(&[97, 98, 99], &[1]), None);
        assert_eq!(find(&[97, 98, 99], &[97]), Some(0));
        assert_eq!(find(&[97, 98, 99], &[97, 98]), Some(0));
    }

    #[test]
    fn test_replace() {
        let mut data = b"abbcb".to_vec();
        replace(&mut data, b'b', b'_');
        assert_eq!(data, b"a__c_");
    }

    #[test]
    fn it_replace_slice() {
        let source = "aba".as_bytes();
        assert_eq!(replace_slice(source, &[97], &[99]), "cbc".as_bytes());
        assert_eq!(replace_slice(source, &[97, 98], &[]), "a".as_bytes());

        let source = "hello\r\nthere\r\n".as_bytes();
        assert_eq!(replace_slice(source, b"\r\n", b""), "hellothere".as_bytes());

        let source = "hello there".as_bytes();
        assert_eq!(
            replace_slice(source, b"\r\n", b""),
            "hello there".as_bytes()
        );
    }
}
