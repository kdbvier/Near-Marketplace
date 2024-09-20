// Find all our documentation at https://docs.near.org
use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::collections::UnorderedMap;
use near_sdk::{env, NearToken, Gas, near_bindgen, AccountId, Promise, serde_json::json, require};
use near_sdk::json_types::U128;
// use near_contract_standards::fungible_token::core_impl::FungibleToken;

// Define the contract structure
#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize)]
#[borsh(crate = "near_sdk::borsh")]
pub struct Contract {
    pub owner_contract: AccountId,
    pub ft_contract:  Option<AccountId>,
    pub amount: u128,
    pub treasury: AccountId,
    pub owned_fts: UnorderedMap<AccountId, u128>
}

impl Default for Contract {
    fn default() -> Self {
        Self {
            owner_contract: env::predecessor_account_id(),
            ft_contract: None, // You need to specify the default value for ft_contract
            amount: 0, // You need to specify the default value for amount
            treasury: env::predecessor_account_id(),
            owned_fts: UnorderedMap::new(b"o")
        }
    }
}

// Implement the contract structure
#[near_bindgen]
impl Contract {
    #[init]
    pub fn init(ft_contract: Option<AccountId>, treasury: AccountId) -> Self {
        if ft_contract.clone().is_some() {
            Promise::new(ft_contract.clone().unwrap()).function_call(
                "storage_deposit".to_string(), 
                json!({
                    "account_id": env::current_account_id()
                }).to_string().into_bytes().to_vec(),
                NearToken::from_millinear(100), 
                Gas::from_tgas(20)
            );
        }
        Self {
            owner_contract: env::predecessor_account_id(),
            ft_contract,
            amount: 0,
            treasury: treasury,
            owned_fts: UnorderedMap::new(b"o")
        }

    }
    
    #[payable]
    pub fn deposit_near(
        &mut self
    ) {
        let attached_amount = env::attached_deposit();
        self.amount = attached_amount.as_yoctonear();
    }
    #[payable]
    pub fn withdraw(
        &mut self,      
        owner: AccountId,
        burn_fee: U128,
    ) {
        assert_eq!(
            env::predecessor_account_id(),
            self.owner_contract,
            "Only the owner contract can withdraw"
        );
        let treasury = &self.treasury;
        let amount_to_holders: u128 = self.amount
            .checked_mul(burn_fee.0).unwrap()
            .checked_div(100u128).unwrap();
        let amount_to_owner = self.amount.checked_sub(amount_to_holders).unwrap();
        if let Some(ft_contract) = &self.ft_contract {
            Promise::new(ft_contract.clone()).function_call(
                "storage_withdraw".to_string(),
                json!({}).to_string().into_bytes().to_vec(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(20),
            ).then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(10))
                    .delete_account(owner.clone())
            );
            Promise::new(ft_contract.clone()).function_call(
                "ft_transfer".to_string(), 
                json!({
                    "receiver_id": owner.clone().to_string(),
                    "amount": amount_to_owner.to_string(),                    
                }).to_string().into_bytes().to_vec(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(20),
            );
            Promise::new(ft_contract.clone()).function_call(
                "ft_transfer".to_string(), 
                json!({
                    "receiver_id": self.owner_contract.to_string(),
                    "amount": (amount_to_holders/2).to_string(),                    
                }).to_string().into_bytes().to_vec(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(20),
            );
            Promise::new(ft_contract.clone()).function_call(
                "ft_transfer".to_string(), 
                json!({
                    "receiver_id": treasury.clone().to_string(),
                    "amount": (amount_to_holders/2).to_string(),                    
                }).to_string().into_bytes().to_vec(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(20),
            );
        } else {
            Promise::new(owner.clone()).transfer(NearToken::from_yoctonear(amount_to_owner));
            Promise::new(self.owner_contract.clone()).transfer(NearToken::from_yoctonear(amount_to_holders/2));
            Promise::new(treasury.clone()).transfer(NearToken::from_yoctonear(amount_to_holders/2));
            Promise::new(env::current_account_id()).delete_account(owner.clone());
        }

        for (owned_ft,_amount) in self.owned_fts.iter() {
            Promise::new(owned_ft).function_call(
                "ft_transfer".to_string(), 
                json!({
                    "receiver_id": owner.to_string(),
                    "amount": _amount.to_string(),                    
                }).to_string().into_bytes().to_vec(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(20),
            );
        }
        self.amount = 0;
    }

    #[private]
    pub fn delete_account(
        &mut self,
        owner: AccountId
    ) -> Promise {
        Promise::new(env::current_account_id()).delete_account(owner)
    }
}


trait FungibleTokenReceiver {
    fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
    ) -> U128;
}

#[near_bindgen]
impl FungibleTokenReceiver for Contract {
    fn ft_on_transfer(
        &mut self,
        _sender_id: AccountId,
        amount: U128,
    ) -> U128 {
        // get the contract ID which is the predecessor
        let ft_contract_id = env::predecessor_account_id();
        if ft_contract_id == self.ft_contract.clone().unwrap() {
            //get the signer which is the person who initiated the transaction
            let signer_id = env::signer_account_id();

            //make sure that the signer isn't the predecessor. This is so that we're sure
            //this was called via a cross-contract call
            assert_ne!(
                ft_contract_id,
                signer_id,
                "ft_on_transfer should only be called via cross-contract call"
            );
            self.amount = self.amount + amount.0;
        } else {
            let prev_amount = self.owned_fts.get(&ft_contract_id).unwrap_or(0);
            let new_amount = prev_amount + amount.0;
            self.owned_fts.insert(&ft_contract_id, &new_amount);
        }
        U128(0)
    }
}