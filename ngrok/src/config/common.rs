use std::{
    collections::HashMap,
    env,
    process,
};

use async_trait::async_trait;
use once_cell::sync::OnceCell;

pub use crate::internals::proto::ProxyProto;
use crate::{
    internals::proto::{
        BindExtra,
        BindOpts,
        IpRestriction,
        MutualTls,
        UserAgentFilter,
    },
    session::RpcError,
    Session,
    Tunnel,
};

pub(crate) fn default_forwards_to() -> &'static str {
    static FORWARDS_TO: OnceCell<String> = OnceCell::new();

    FORWARDS_TO
        .get_or_init(|| {
            let hostname = hostname::get()
                .unwrap_or("<unknown>".into())
                .to_string_lossy()
                .into_owned();
            let exe = env::current_exe()
                .unwrap_or("<unknown>".into())
                .to_string_lossy()
                .into_owned();
            let pid = process::id();
            format!("app://{hostname}/{exe}?pid={pid}")
        })
        .as_str()
}

/// Trait representing things that can be built into an ngrok tunnel.
#[async_trait]
pub trait TunnelBuilder: From<Session> {
    /// The ngrok tunnel type that this builder produces.
    type Tunnel: Tunnel;

    /// Begin listening for new connections on this tunnel.
    async fn listen(&self) -> Result<Self::Tunnel, RpcError>;
}

macro_rules! impl_builder {
    ($(#[$m:meta])* $name:ident, $opts:ty, $tun:ident) => {
        $(#[$m])*
        #[derive(Clone)]
        pub struct $name {
            options: $opts,
            // Note: This is only optional for testing purposes.
            session: Option<Session>,
        }

        impl From<Session> for $name {
            fn from(session: Session) -> Self {
                $name {
                    options: Default::default(),
                    session: session.into(),
                }
            }
        }

        #[async_trait]
        impl TunnelBuilder for $name {
            type Tunnel = $tun;

            async fn listen(&self) -> Result<$tun, RpcError> {
                Ok($tun {
                    inner: self
                        .session
                        .as_ref()
                        .unwrap()
                        .start_tunnel(&self.options)
                        .await?,
                })
            }
        }
    };
}
/// Tunnel configuration trait, implemented by our top-level config objects.
///
/// "Sealed," i.e. not implementable outside of the crate.
pub(crate) trait TunnelConfig {
    /// The "forwards to" metadata.
    ///
    /// Only for display/informational purposes.
    fn forwards_to(&self) -> String;
    /// Internal-only, extra data sent when binding a tunnel.
    fn extra(&self) -> BindExtra;
    /// The protocol for this tunnel.
    fn proto(&self) -> String;
    /// The middleware and other configuration options for this tunnel.
    fn opts(&self) -> Option<BindOpts>;
    /// The labels for this tunnel.
    fn labels(&self) -> HashMap<String, String>;
}

// delegate references
impl<'a, T> TunnelConfig for &'a T
where
    T: TunnelConfig,
{
    fn forwards_to(&self) -> String {
        (**self).forwards_to()
    }
    fn extra(&self) -> BindExtra {
        (**self).extra()
    }
    fn proto(&self) -> String {
        (**self).proto()
    }
    fn opts(&self) -> Option<BindOpts> {
        (**self).opts()
    }
    fn labels(&self) -> HashMap<String, String> {
        (**self).labels()
    }
}

/// Restrictions placed on the origin of incoming connections to the edge.
#[derive(Clone, Default)]
pub(crate) struct CidrRestrictions {
    /// Rejects connections that do not match the given CIDRs
    pub(crate) allowed: Vec<String>,
    /// Rejects connections that match the given CIDRs and allows all other CIDRs.
    pub(crate) denied: Vec<String>,
}

impl CidrRestrictions {
    pub(crate) fn allow(&mut self, cidr: impl Into<String>) {
        self.allowed.push(cidr.into());
    }
    pub(crate) fn deny(&mut self, cidr: impl Into<String>) {
        self.denied.push(cidr.into());
    }
}

/// Restrictions placed on the origin of incoming connections to the edge.
#[derive(Clone, Default)]
pub(crate) struct UaFilter {
    /// Rejects connections that do not match the given regular expression
    pub(crate) allow: Vec<String>,
    /// Rejects connections that match the given regular expression and allows all other regular
    /// expressions.
    pub(crate) deny: Vec<String>,
}

impl UaFilter {
    pub(crate) fn allow(&mut self, allow: impl Into<String>) {
        self.allow.push(allow.into());
    }
    pub(crate) fn deny(&mut self, deny: impl Into<String>) {
        self.deny.push(deny.into());
    }
}

// Common
#[derive(Default, Clone)]
pub(crate) struct CommonOpts {
    // Restrictions placed on the origin of incoming connections to the edge.
    pub(crate) cidr_restrictions: CidrRestrictions,
    // The version of PROXY protocol to use with this tunnel, zero if not
    // using.
    pub(crate) proxy_proto: ProxyProto,
    // Tunnel-specific opaque metadata. Viewable via the API.
    pub(crate) metadata: Option<String>,
    // Tunnel backend metadata. Viewable via the dashboard and API, but has no
    // bearing on tunnel behavior.
    pub(crate) forwards_to: Option<String>,
    // Flitering placed on the origin of incoming connections to the edge.
    pub(crate) user_agent_filter: UaFilter,
}

impl CommonOpts {
    // Get the proto version of cidr restrictions
    pub(crate) fn ip_restriction(&self) -> Option<IpRestriction> {
        (!self.cidr_restrictions.allowed.is_empty() || !self.cidr_restrictions.denied.is_empty())
            .then_some(self.cidr_restrictions.clone().into())
    }
    pub(crate) fn user_agent_filter(&self) -> Option<UserAgentFilter> {
        (!self.user_agent_filter.allow.is_empty() || !self.user_agent_filter.deny.is_empty())
            .then_some(self.user_agent_filter.clone().into())
    }
}

// transform into the wire protocol format
impl From<CidrRestrictions> for IpRestriction {
    fn from(cr: CidrRestrictions) -> Self {
        IpRestriction {
            allow_cidrs: cr.allowed,
            deny_cidrs: cr.denied,
        }
    }
}

impl From<UaFilter> for UserAgentFilter {
    fn from(ua: UaFilter) -> Self {
        UserAgentFilter {
            allow: ua.allow,
            deny: ua.deny,
        }
    }
}

impl From<&[bytes::Bytes]> for MutualTls {
    fn from(b: &[bytes::Bytes]) -> Self {
        let mut aggregated = Vec::new();
        b.iter().for_each(|c| aggregated.extend(c));
        MutualTls {
            mutual_tls_ca: aggregated,
        }
    }
}
