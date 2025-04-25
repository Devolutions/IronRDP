// QUESTION: consider auto-generating this file based on a reference file?
// https://gist.github.com/awakecoding/838c7fe2ed3a6208e3ca5d8af25363f6

use ironrdp_propertyset::PropertySet;

pub trait PropertySetExt {
    fn full_address(&self) -> Option<&str>;

    fn server_port(&self) -> Option<i64>;

    fn alternate_full_address(&self) -> Option<&str>;

    fn gateway_hostname(&self) -> Option<&str>;

    fn remote_application_name(&self) -> Option<&str>;

    fn remote_application_program(&self) -> Option<&str>;

    fn kdc_proxy_url(&self) -> Option<&str>;

    fn username(&self) -> Option<&str>;

    /// Target RDP server password - use for testing only
    fn clear_text_password(&self) -> Option<&str>;
}

impl PropertySetExt for PropertySet {
    fn full_address(&self) -> Option<&str> {
        self.get::<&str>("full address")
    }

    fn server_port(&self) -> Option<i64> {
        self.get::<i64>("server port")
    }

    fn alternate_full_address(&self) -> Option<&str> {
        self.get::<&str>("alternate full address")
    }

    fn gateway_hostname(&self) -> Option<&str> {
        self.get::<&str>("gatewayhostname")
    }

    fn remote_application_name(&self) -> Option<&str> {
        self.get::<&str>("remoteapplicationname")
    }

    fn remote_application_program(&self) -> Option<&str> {
        self.get::<&str>("remoteapplicationprogram")
    }

    fn kdc_proxy_url(&self) -> Option<&str> {
        self.get::<&str>("kdcproxyurl")
    }

    fn username(&self) -> Option<&str> {
        self.get::<&str>("username")
    }

    fn clear_text_password(&self) -> Option<&str> {
        self.get::<&str>("ClearTextPassword")
    }
}
