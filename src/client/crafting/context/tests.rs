use super::*;
use crate::{MinecraftVersion, client::crafting::test_receipts as receipts};
#[test]
fn crafting_context_seals_all_capture_boundaries_and_preserves_historical_owners() {
    for version in [MinecraftVersion::Java1_16_1, MinecraftVersion::Java1_21_11] {
        let player = receipts::player(version);
        let registry = receipts::registries(&player);
        let recipes = receipts::recipes(&player, receipts::display(version, 1, 1, true, 1));
        let screen = receipts::screen(&player, false, &[]);
        let context = ReceivedCraftingContext::capture(
            player.clone(),
            screen.clone(),
            registry.clone(),
            recipes.clone(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(context.player().session, context.inventory().session());
        assert_eq!(context.grid().session(), context.recipes().session());
        assert_eq!(
            context.grid().receive_sequence(),
            context.recipes().receive_sequence()
        );
        assert_eq!(
            context.inventory().registry_state().stamp(),
            context.recipes().registry_state().stamp()
        );
        let mut other = player.clone();
        other.session.connection_id += 1;
        assert!(
            ReceivedCraftingContext::capture(
                other,
                screen.clone(),
                registry.clone(),
                recipes.clone()
            )
            .is_err()
        );
        let mut other = player.clone();
        other.receive_sequence += 1;
        assert!(
            ReceivedCraftingContext::capture(
                other,
                screen.clone(),
                registry.clone(),
                recipes.clone()
            )
            .is_err()
        );
        let mut other = screen.clone();
        other.receive_sequence += 1;
        assert!(
            ReceivedCraftingContext::capture(
                player.clone(),
                other,
                registry.clone(),
                recipes.clone()
            )
            .is_err()
        );
        let mut other = screen.clone();
        other.active_window = Some(3);
        assert!(
            ReceivedCraftingContext::capture(
                player.clone(),
                other,
                registry.clone(),
                recipes.clone()
            )
            .is_err()
        );
        let mut other = player.clone();
        other.inventory.player_screen = None;
        other.inventory.window_id = Some(3);
        assert!(
            ReceivedCraftingContext::capture(
                other.clone(),
                receipts::screen(&other, false, &[]),
                registry.clone(),
                recipes.clone()
            )
            .unwrap()
            .is_none()
        );
        assert_eq!(context.session(), player.session);
        assert_eq!(context.receive_sequence(), 20);
    }
}
