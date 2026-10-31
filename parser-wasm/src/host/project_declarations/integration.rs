//! Connection of immutable prototypes to the existing document transaction.
use super::*;

impl ParserHost {
    /// Whether any loaded provider requires the project header protocol.
    pub fn project_declarations_enabled(&self) -> bool {
        self.components.iter().any(|entry| component_promises(&entry.manifest.capabilities, protocol::CAPABILITY))
    }

    /// Refresh source membership before the coordinator lends cached results.
    /// This never publishes declarations or revives a superseded source view.
    pub fn synchronize_declaration_sources(&mut self, request: &DocumentParseRequest,
        inputs: &DeclarationInputs) -> Result<(), HostError>
    {
        self.project_sources.prepare(request, inputs).map(|_| ())
    }

    /// Revalidate the complete source/provider fence, including foreign inputs.
    pub fn declaration_exports_are_current(&self, project: &str, exports: &DeclarationExports) -> bool {
        exports.current(project, &self.project_sources, &ProviderStamp::capture(self))
    }

    pub(in crate::host) fn begin_project_declarations(&mut self, request: &DocumentParseRequest,
        inputs: &DeclarationInputs) -> Result<(), HostError>
    {
        let routes = self.registry.subscriptions.iter().filter(|route|
            route.subscription.capability_id == crate::CAPABILITY_DOCUMENT_DECLARATIONS
                && component_promises(&self.components[route.component_index].manifest.capabilities, protocol::CAPABILITY))
            .map(|route| (route.component_index, route.subscription.id.clone())).collect::<Vec<_>>();
        if routes.is_empty() { return Ok(()); }
        // Stabilize source preparation generations before binding any input.
        for index in (0..self.components.len()).filter(|index| !self.components[*index].disabled && !self.components[*index].unloaded).collect::<Vec<_>>() {
            self.ensure_component_ready(index)?;
        }
        let expected = routes.into_iter().map(|(index, hook)|
            (self.components[index].manifest.component_id.clone(), hook)).collect();
        let view = self.project_sources.prepare(request, inputs)?;
        self.project_declarations = Some(Arc::new(Mutex::new(Arena::new(request.project_uri.clone(),
            view, ProviderStamp::capture(self), inputs, expected))));
        Ok(())
    }

    pub(in crate::host) fn project_declaration_body_key(&self) -> String {
        self.project_declarations.as_ref().and_then(|frame| frame.lock().ok().map(|frame| frame.body_key.clone()))
            .unwrap_or_default()
    }

}
