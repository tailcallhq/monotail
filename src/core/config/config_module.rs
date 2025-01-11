use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Deref;

use jsonwebtoken::jwk::JwkSet;
use prost_reflect::prost_types::{FileDescriptorProto, FileDescriptorSet};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tailcall_valid::{Valid, Validator};

use super::LinkType;
use crate::core::config::Config;
use crate::core::macros::MergeRight;
use crate::core::merge_right::MergeRight;
use crate::core::proto_reader::ProtoMetadata;
use crate::core::rest::{EndpointSet, Unchecked};
use crate::core::Transform;

mod merge;

/// A wrapper on top of Config that contains all the resolved extensions and
/// computed values.
#[derive(Clone, Debug, Default)]
pub struct ConfigModule {
    extensions: Extensions,
    cache: Cache,
}

/// A cache that store resolved input, output and interface types so that it's
/// not computed again and again.
#[derive(Clone, Debug, Default)]
struct Cache {
    config: Config,
    input_types: HashSet<String>,
    output_types: HashSet<String>,
    interfaces_types_map: BTreeMap<String, BTreeSet<String>>,
}

impl From<Config> for Cache {
    fn from(value: Config) -> Self {
        let input_types = value.input_types();
        let output_types = value.output_types();
        let interfaces_types_map = value.interfaces_types_map();

        Cache {
            config: value,
            input_types,
            output_types,
            interfaces_types_map,
        }
    }
}

impl ConfigModule {
    pub fn new(config: Config, extensions: Extensions) -> Self {
        ConfigModule { cache: Cache::from(config), extensions }
    }

    /// Identifies if an enterprise feature is being used
    pub fn is_enterprise_features_enabled(&self) -> bool {
        let js_capability_enabled = self
            .config()
            .links
            .iter()
            .any(|link| matches!(link.type_of, LinkType::Script));
        let telemetry_capability_enabled = self.config().telemetry.is_enabled();
        js_capability_enabled || telemetry_capability_enabled
    }

    pub fn set_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = extensions;
        self
    }

    pub fn merge_extensions(mut self, extensions: Extensions) -> Self {
        self.extensions = self.extensions.merge_right(extensions);
        self
    }

    pub fn config(&self) -> &Config {
        &self.cache.config
    }

    pub fn extensions(&self) -> &Extensions {
        &self.extensions
    }

    pub fn input_types(&self) -> &HashSet<String> {
        &self.cache.input_types
    }

    pub fn output_types(&self) -> &HashSet<String> {
        &self.cache.output_types
    }

    pub fn interfaces_types_map(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.cache.interfaces_types_map
    }

    pub fn transform<T: Transform<Value = Config>>(self, transformer: T) -> Valid<Self, T::Error> {
        transformer
            .transform(self.cache.config)
            .map(|config| ConfigModule::new(config, self.extensions))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Content<A> {
    pub id: Option<String>,
    pub content: A,
}

impl<A> Deref for Content<A> {
    type Target = A;
    fn deref(&self) -> &Self::Target {
        &self.content
    }
}

#[derive(Debug)]
pub struct PrivateKey(PrivateKeyDer<'static>);

impl Clone for PrivateKey {
    fn clone(&self) -> Self {
        Self(self.0.clone_key())
    }
}

impl From<PrivateKeyDer<'static>> for PrivateKey {
    fn from(value: PrivateKeyDer<'static>) -> Self {
        Self(value)
    }
}

impl PrivateKey {
    pub fn into_inner(self) -> PrivateKeyDer<'static> {
        self.0
    }
}

/// Extensions are meta-information required before we can generate the
/// blueprint. Typically, this information cannot be inferred without performing
/// an IO operation, i.e., reading a file, making an HTTP call, etc.
#[derive(Clone, Debug, Default, MergeRight)]
pub struct Extensions {
    /// Contains the file descriptor set resolved from the links to proto files
    pub grpc_file_descriptors: HashMap<String, FileDescriptorProto>,

    /// Contains the contents of the JS file
    pub script: Option<String>,

    /// Contains the certificate used on HTTP2 with TLS
    pub cert: Vec<CertificateDer<'static>>,

    /// Contains the key used on HTTP2 with TLS
    pub keys: Vec<PrivateKey>,

    /// Contains the endpoints
    pub endpoint_set: EndpointSet<Unchecked>,

    pub htpasswd: Vec<Content<String>>,

    pub jwks: Vec<Content<JwkSet>>,
}

impl Extensions {
    pub fn add_proto(&mut self, metadata: ProtoMetadata) {
        for file in metadata.descriptor_set.file {
            self.grpc_file_descriptors
                .insert(file.name().to_string(), file);
        }
    }

    pub fn get_file_descriptor_set(&self) -> FileDescriptorSet {
        FileDescriptorSet { file: self.grpc_file_descriptors.values().cloned().collect() }
    }

    pub fn has_auth(&self) -> bool {
        !self.htpasswd.is_empty() || !self.jwks.is_empty()
    }
}

impl MergeRight for FileDescriptorSet {
    fn merge_right(mut self, other: Self) -> Self {
        self.file.extend(other.file);

        self
    }
}

impl Deref for ConfigModule {
    type Target = Config;
    fn deref(&self) -> &Self::Target {
        self.config()
    }
}

impl From<Config> for ConfigModule {
    fn from(config: Config) -> Self {
        ConfigModule { cache: Cache::from(config), ..Default::default() }
    }
}
