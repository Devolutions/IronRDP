// QUESTION: consider auto-generating this file based on a reference file?
// https://gist.github.com/awakecoding/838c7fe2ed3a6208e3ca5d8af25363f6

use ironrdp_propertyset::{ExtractFrom, PropertySet, Value};

pub trait PropertySetExt {
    fn read<'a, V: ExtractFrom<&'a Value>>(&'a self, key: &str) -> Option<V>;

    fn full_address(&self) -> Option<&str> {
        self.read::<&str>("full address")
    }

    fn alternate_full_address(&self) -> Option<&str> {
        self.read::<&str>("alternate full address")
    }

    fn gateway_hostname(&self) -> Option<&str> {
        self.read::<&str>("gatewayhostname")
    }

    fn remote_application_name(&self) -> Option<&str> {
        self.read::<&str>("remoteapplicationname")
    }

    fn remote_application_program(&self) -> Option<&str> {
        self.read::<&str>("remoteapplicationprogram")
    }

    fn kdc_proxy_url(&self) -> Option<&str> {
        self.read::<&str>("kdcproxyurl")
    }
}

impl PropertySetExt for PropertySet {
    fn read<'a, V: ExtractFrom<&'a Value>>(&'a self, key: &str) -> Option<V> {
        todo!("should we log directly from ironrdp-propertyset?");
    }
}
