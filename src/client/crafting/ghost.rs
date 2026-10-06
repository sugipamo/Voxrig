//! Actual ghost UI packets, kept separate from input and inventory receipts.
use super::{CraftingSource, RecipeDisplay, RecipeId, recipes::RecipeReceipts};
use crate::{
    Result,
    client::{
        SessionStamp,
        container::{ContainerCloseRecord, ScreenReceipts, player_screen_access},
        registry::{ServerRegistryObservation, received::ReceivedRegistries},
    },
};

/// Historical server ghost display on one actual crafting UI. This is not an
/// input stack, crafted result, placement acknowledgement or replay permission.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ReceivedRecipeGhost {
    session: SessionStamp,
    receive_sequence: u64,
    source: CraftingSource,
    recipe: Option<RecipeId>,
    recipe_name: Option<String>,
    display: Option<RecipeDisplay>,
    registries: ServerRegistryObservation,
}
impl ReceivedRecipeGhost {
    /// Original transport/world.
    pub fn session(&self) -> SessionStamp {
        self.session
    }
    /// Actual ghost packet ordinal, separate from slot receipt ordinals.
    pub fn receive_sequence(&self) -> u64 {
        self.receive_sequence
    }
    /// Original UI, including its opening or explicit submitted-close basis.
    pub fn source(&self) -> CraftingSource {
        self.source
    }
    /// Actual declared identity when the native response supplies a recipe name.
    /// Display-only responses provide no recipe ID; no identity is inferred.
    pub fn recipe(&self) -> Option<&RecipeId> {
        self.recipe.as_ref()
    }
    /// Native response name, absent when the protocol sends only a display.
    pub fn recipe_name(&self) -> Option<&str> {
        self.recipe_name.as_deref()
    }
    /// Received display, or the original declaration selected by the received name.
    /// None means that declaration was missing, not an empty recipe.
    pub fn display(&self) -> Option<&RecipeDisplay> {
        self.display.as_ref()
    }
    /// Frozen registry/tag context at the ghost packet boundary.
    pub fn registry_state(&self) -> &ServerRegistryObservation {
        &self.registries
    }
}

#[derive(Clone, Debug)]
pub(crate) struct GhostContext {
    pub generation: u64,
    pub sequence: u64,
    pub active_window: Option<i32>,
    pub screen: Option<ScreenReceipts>,
    pub close: Option<ContainerCloseRecord>,
    pub registries: ReceivedRegistries,
}
#[derive(Clone, Debug)]
enum NativeGhost {
    Named {
        name: String,
        declaration: Option<(u64, RecipeDisplay)>,
    },
    Display(RecipeDisplay),
}
#[derive(Clone, Debug)]
pub(crate) struct GhostReceipts {
    context: GhostContext,
    window: i32,
    native: NativeGhost,
}
impl GhostReceipts {
    pub(crate) fn receive_sequence(&self) -> u64 {
        self.context.sequence
    }
    pub(crate) fn named(
        context: GhostContext,
        window: i32,
        name: String,
        recipes: &RecipeReceipts,
    ) -> Self {
        let declaration = recipes.ghost_declaration(&name);
        Self {
            context,
            window,
            native: NativeGhost::Named { name, declaration },
        }
    }
    pub(crate) fn displayed(context: GhostContext, window: i32, display: RecipeDisplay) -> Self {
        Self {
            context,
            window,
            native: NativeGhost::Display(display),
        }
    }
    pub(crate) fn capture(&self, session: SessionStamp) -> Result<Option<ReceivedRecipeGhost>> {
        if session.world_generation != self.context.generation {
            return Ok(None);
        }
        let screen = self.context.screen.as_ref().map(|s| s.capture(session));
        let source = if self.window == 0 {
            let Some(access) = player_screen_access(
                session,
                self.context.active_window,
                screen.as_ref().map(|s| s.id),
                self.context.close.as_ref(),
            ) else {
                return Ok(None);
            };
            CraftingSource::Player { access }
        } else {
            let Some(screen) = screen.filter(|s| {
                s.id.window_id() == self.window
                    && self.context.active_window == Some(self.window)
                    && s.menu_name.as_deref() == Some("minecraft:crafting")
            }) else {
                return Ok(None);
            };
            CraftingSource::Table { screen: screen.id }
        };
        let registries = self
            .context
            .registries
            .capture(session, self.context.sequence);
        let (recipe, recipe_name, mut display) = match &self.native {
            NativeGhost::Named { name, declaration } => (
                declaration.as_ref().map(|(ordinal, _)| {
                    RecipeId::declared_ghost(name.clone(), *ordinal, registries.stamp())
                }),
                Some(name.clone()),
                declaration.as_ref().map(|(_, display)| display.clone()),
            ),
            NativeGhost::Display(display) => (None, None, Some(display.clone())),
        };
        if let Some(display) = display.as_mut() {
            display.bind(&registries)?;
        }
        Ok(Some(ReceivedRecipeGhost {
            session,
            receive_sequence: self.context.sequence,
            source,
            recipe,
            recipe_name,
            display,
            registries,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MinecraftVersion, client::container::ScreenTitle};
    #[test]
    fn ghost_keeps_original_opening_registry_boundary_and_world() {
        let version = MinecraftVersion::Java1_21_11;
        let session = SessionStamp {
            version,
            connection_id: 7,
            world_generation: 3,
        };
        let mut registries = ReceivedRegistries::default();
        registries.finish();
        let mut screen = ScreenReceipts::open(
            version,
            3,
            None,
            ScreenTitle::LegacyJson { json: "{}".into() },
            12,
        );
        screen.menu_name = Some("minecraft:crafting".into());
        let context = GhostContext {
            generation: 3,
            sequence: 20,
            active_window: Some(3),
            screen: Some(screen),
            close: None,
            registries,
        };
        let mut later = context.clone();
        let ghost = GhostReceipts::displayed(
            context,
            3,
            RecipeDisplay::Shapeless {
                ingredients: vec![],
                result: super::super::RecipeSlotDisplay::Empty,
                crafting_station: Some(super::super::RecipeSlotDisplay::Empty),
            },
        );
        let original = ghost.capture(session).unwrap().unwrap();
        later.screen.as_mut().unwrap().opened_sequence = 30;
        later.sequence = 40;
        later.registries.reset(35);
        let replacement = GhostReceipts::displayed(later, 3, original.display().unwrap().clone())
            .capture(session)
            .unwrap()
            .unwrap();
        assert_ne!(original.source(), replacement.source());
        assert_ne!(
            original.registry_state().stamp(),
            replacement.registry_state().stamp()
        );
        assert_eq!(
            ghost.capture(session).unwrap().unwrap().source(),
            original.source()
        );
        assert_eq!(original.receive_sequence(), 20);
        assert!(
            ghost
                .capture(SessionStamp {
                    world_generation: 4,
                    ..session
                })
                .unwrap()
                .is_none()
        );
    }
}
