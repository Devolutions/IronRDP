#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerName(String);

impl ServerName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(sanitize_server_name(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl From<String> for ServerName {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&String> for ServerName {
    fn from(value: &String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ServerName {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

fn sanitize_server_name(name: impl Into<String>) -> String {
    let name = name.into();

    if let Some(idx) = name.rfind(':') {
        if let Ok(sock_addr) = name.parse::<std::net::SocketAddr>() {
            // A socket address, including a port
            sock_addr.ip().to_string()
        } else if name.parse::<std::net::Ipv6Addr>().is_ok() {
            // An IPv6 address with no port, do not include a port, already sane
            name
        } else {
            // An IPv4 address or server hostname including a port after the `:` token
            name[..idx].to_owned()
        }
    } else {
        // An IPv4 address or server hostname which does not include a port, already sane
        name
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("somehostname:2345", "somehostname")]
    #[case("192.168.56.101:2345", "192.168.56.101")]
    #[case("[2001:db8::8a2e:370:7334]:7171", "2001:db8::8a2e:370:7334")]
    #[case("[2001:0db8:0000:0000:0000:8a2e:0370:7334]:433", "2001:db8::8a2e:370:7334")]
    #[case("[::1]:2222", "::1")]
    fn input_with_port(#[case] input: &str, #[case] expected: &str) {
        let result = sanitize_server_name(input);
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case("somehostname")]
    #[case("192.168.56.101")]
    #[case("2001:db8::8a2e:370:7334")]
    #[case("2001:0db8:0000:0000:0000:8a2e:0370:7334")]
    #[case("::1")]
    fn input_without_port(#[case] input: &str) {
        let result = sanitize_server_name(input);
        assert_eq!(result, input);
    }
}
