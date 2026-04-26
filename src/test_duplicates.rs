#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::Address as _,
    Address, Env, symbol_short,
};

fn setup_test() -> (Env, RevoraRevenueShareClient<'static>, Address) {
    let env = Env::default();
    env.mock_all_auths();
    
    let contract_id = env.register_contract(None, RevoraRevenueShare);
    let client = RevoraRevenueShareClient::new(&env, &contract_id);
    
    let admin = Address::generate(&env);
    client.initialize(&admin, &None, &None);
    
    (env, client, admin)
}

#[test]
fn test_register_duplicate_offering_is_idempotent() {
    let (env, client, issuer) = setup_test();
    let token = Address::generate(&env);
    let namespace = symbol_short!("ns");
    let payout_asset = Address::generate(&env);
    
    // First registration
    client.register_offering(&issuer, &namespace, &token, &5000, &payout_asset, &0);
    assert_eq!(client.get_offering_count(&issuer, &namespace), 1);
    
    let offering1 = client.get_offering(&issuer, &namespace, &token).unwrap();
    assert_eq!(offering1.revenue_share_bps, 5000);
    
    // Second registration (same identity, different bps)
    client.register_offering(&issuer, &namespace, &token, &6000, &payout_asset, &0);
    
    // Count should still be 1
    assert_eq!(client.get_offering_count(&issuer, &namespace), 1);
    
    // Offering parameters should NOT have changed (preserving original)
    let offering2 = client.get_offering(&issuer, &namespace, &token).unwrap();
    assert_eq!(offering2.revenue_share_bps, 5000);
}

#[test]
fn test_pagination_stability_with_idempotency() {
    let (env, client, issuer) = setup_test();
    let namespace = symbol_short!("ns");
    let payout_asset = Address::generate(&env);
    
    let token_a = Address::generate(&env);
    let token_b = Address::generate(&env);
    
    // Register A, then B, then A again
    client.register_offering(&issuer, &namespace, &token_a, &1000, &payout_asset, &0);
    client.register_offering(&issuer, &namespace, &token_b, &2000, &payout_asset, &0);
    client.register_offering(&issuer, &namespace, &token_a, &3000, &payout_asset, &0);
    
    let (offerings, _) = client.get_offerings_page(&issuer, &namespace, &0, &10);
    
    // Should only have 2 unique offerings in the order they were first registered
    assert_eq!(offerings.len(), 2);
    assert_eq!(offerings.get(0).unwrap().token, token_a);
    assert_eq!(offerings.get(1).unwrap().token, token_b);
}

#[test]
fn test_get_offering_matches_first_registration() {
    let (env, client, issuer) = setup_test();
    let token = Address::generate(&env);
    let namespace = symbol_short!("ns");
    let payout_asset = Address::generate(&env);
    
    client.register_offering(&issuer, &namespace, &token, &1000, &payout_asset, &0);
    client.register_offering(&issuer, &namespace, &token, &2000, &payout_asset, &0);
    
    let offering = client.get_offering(&issuer, &namespace, &token).unwrap();
    assert_eq!(offering.revenue_share_bps, 1000);
}
