use std::collections::HashSet;

use indexmap::{IndexMap, IndexSet};
use inflector::Inflector;

use crate::core::config::Config;
use crate::core::transform::Transform;
use crate::core::valid::Valid;

#[derive(Debug, Default)]
struct CandidateStats {
    frequency: u32,
    priority: u8,
}

struct CandidateConvergence<'a> {
    /// maintains the generated candidates in the form of
    /// {TypeName: {{candidate_name: {frequency: 1, priority: 0}}}}
    candidates: IndexMap<String, IndexMap<String, CandidateStats>>,
    config: &'a Config,
}

impl<'a> CandidateConvergence<'a> {
    fn new(candate_gen: CandidateGeneration<'a>) -> Self {
        Self {
            candidates: candate_gen.candidates,
            config: candate_gen.config,
        }
    }

    /// Converges on the most frequent candidate name for each type.
    /// This method selects the most frequent candidate name for each type,
    /// ensuring uniqueness.
    fn converge(self) -> IndexMap<String, String> {
        let mut finalized_candidates = IndexMap::new();
        let mut converged_candidate_set = HashSet::new();

        for (type_name, candidate_list) in self.candidates.iter() {
            // Filter out candidates that have already been converged or are already present
            // in types
            let candidates_to_consider = candidate_list.iter().filter(|(candidate_name, _)| {
                let candidate_type_name = candidate_name.to_pascal_case();
                !converged_candidate_set.contains(&candidate_type_name)
                    && !self.config.types.contains_key(&candidate_type_name)
            });

            // Find the candidate with the highest frequency and priority
            if let Some((candidate_name, _)) = candidates_to_consider
                .max_by_key(|(key, value)| (value.priority, value.frequency, *key))
            {
                let singularized_candidate_name = candidate_name.to_pascal_case();
                finalized_candidates
                    .insert(type_name.to_owned(), singularized_candidate_name.clone());
                converged_candidate_set.insert(singularized_candidate_name);
            }
        }

        finalized_candidates
    }
}

struct CandidateGeneration<'a> {
    /// maintains the generated candidates in the form of
    /// {TypeName: {{candidate_name: {frequency: 1, priority: 0}}}}
    candidates: IndexMap<String, IndexMap<String, CandidateStats>>,
    suggested_names: &'a HashSet<String>,
    config: &'a Config,
}

impl<'a> CandidateGeneration<'a> {
    fn new(config: &'a Config, suggested_names: &'a HashSet<String>) -> Self {
        Self { candidates: Default::default(), config, suggested_names }
    }

    /// Generates candidate type names based on the provided configuration.
    /// This method iterates over the configuration and collects candidate type
    /// names for each type.
    fn generate(mut self) -> CandidateConvergence<'a> {
        // process the user suggest field names over the auto inferred names.
        let ty_names = vec![
            self.config.schema.query.as_ref(),
            self.config.schema.mutation.as_ref(),
            self.config.schema.subscription.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(|o| o.to_owned())
        .chain(self.config.types.keys().cloned())
        .collect::<IndexSet<_>>();

        for ref type_name in ty_names {
            if let Some(type_info) = self.config.types.get(type_name) {
                for (field_name, field_info) in type_info.fields.iter() {
                    if self.config.is_scalar(&field_info.type_of) {
                        // If field type is scalar then ignore type name inference.
                        continue;
                    }

                    let inner_map = self
                        .candidates
                        .entry(field_info.type_of.to_owned())
                        .or_default();

                    let singularized_candidate = field_name.to_singular();

                    if let Some(key_val) = inner_map.get_mut(&singularized_candidate) {
                        key_val.frequency += 1
                    } else {
                        let priority = match self.config.is_root_operation_type(type_name) {
                            true => {
                                if self.suggested_names.contains(field_name) {
                                    // priority of user suggested name is higher than anything.
                                    2
                                } else {
                                    0
                                }
                            }
                            false => 1,
                        };

                        println!(
                            "[Finder]: {:#?} and {:#?} and {:#?} and {:#?}",
                            field_name,
                            priority,
                            type_name,
                            self.config.is_root_operation_type(type_name)
                        );
                        inner_map.insert(
                            singularized_candidate,
                            CandidateStats { frequency: 1, priority },
                        );
                    }
                }
            }
        }
        println!("[Finder]: {:#?}", self.candidates);
        CandidateConvergence::new(self)
    }
}

#[derive(Default)]
pub struct ImproveTypeNames {
    // given set of names, transformer prioritizes given names over the frequency in the final
    // config.
    suggested_names: HashSet<String>,
}

impl ImproveTypeNames {
    pub fn new(name: HashSet<String>) -> Self {
        Self { suggested_names: name }
    }

    /// Generates type names based on inferred candidates from the provided
    /// configuration.
    fn generate_type_names(&self, mut config: Config) -> Config {
        let finalized_candidates = CandidateGeneration::new(&config, &self.suggested_names)
            .generate()
            .converge();

        for (old_type_name, new_type_name) in finalized_candidates {
            if let Some(type_) = config.types.remove(old_type_name.as_str()) {
                // Add newly generated type.
                config.types.insert(new_type_name.to_owned(), type_);

                // Replace all the instances of old name in config.
                for actual_type in config.types.values_mut() {
                    for actual_field in actual_type.fields.values_mut() {
                        if actual_field.type_of == old_type_name {
                            // Update the field's type with the new name
                            actual_field.type_of.clone_from(&new_type_name);
                        }
                    }
                }
            }
        }
        config
    }
}

impl Transform for ImproveTypeNames {
    type Value = Config;
    type Error = String;
    fn transform(&self, config: Config) -> Valid<Self::Value, Self::Error> {
        let config = self.generate_type_names(config);

        Valid::succeed(config)
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;
    use std::fs;

    use anyhow::Ok;
    use tailcall_fixtures::configs;

    use super::ImproveTypeNames;
    use crate::core::config::Config;
    use crate::core::transform::Transform;
    use crate::core::valid::Validator;

    fn read_fixture(path: &str) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn test_type_name_generator_transform() {
        let config = Config::from_sdl(read_fixture(configs::AUTO_GENERATE_CONFIG).as_str())
            .to_result()
            .unwrap();

        let transformed_config = ImproveTypeNames::new(HashSet::default())
            .transform(config)
            .to_result()
            .unwrap();
        insta::assert_snapshot!(transformed_config.to_sdl());
    }

    #[test]
    fn test_type_name_generator_with_cyclic_types() -> anyhow::Result<()> {
        let config = Config::from_sdl(read_fixture(configs::CYCLIC_CONFIG).as_str())
            .to_result()
            .unwrap();

        let transformed_config = ImproveTypeNames::new(HashSet::default())
            .transform(config)
            .to_result()
            .unwrap();
        insta::assert_snapshot!(transformed_config.to_sdl());

        Ok(())
    }

    #[test]
    fn test_type_name_generator() -> anyhow::Result<()> {
        let config = Config::from_sdl(read_fixture(configs::NAME_GENERATION).as_str())
            .to_result()
            .unwrap();

        let transformed_config = ImproveTypeNames::new(HashSet::default())
            .transform(config)
            .to_result()
            .unwrap();
        insta::assert_snapshot!(transformed_config.to_sdl());

        Ok(())
    }

    #[test]
    fn test_prioritize_suggested_name() -> anyhow::Result<()> {
        let config = Config::from_sdl(read_fixture(configs::CONFLICTING_TYPE_NAMES).as_str())
            .to_result()
            .unwrap();

        let mut suggested_names = HashSet::default();
        suggested_names.insert("post".to_owned());
        suggested_names.insert("todos".to_owned());
        suggested_names.insert("userPosts".to_owned());

        let transformed_config = ImproveTypeNames::new(suggested_names)
            .transform(config)
            .to_result()
            .unwrap();
        insta::assert_snapshot!(transformed_config.to_sdl());

        Ok(())
    }
}
