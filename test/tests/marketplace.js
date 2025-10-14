const utils = require("./utils");
// const utils = require("./test-utils");

const { mainContract, gas, ownerAccount, userAccount, marketplace } = utils;

async function init() {
  // await ownerAccount.functionCall({
  //   contractId: marketplace.contractId,
  //   methodName: "new", 
  //   args: {
  //     owner_id: ownerAccount.accountId,
  //     treasury_id: ownerAccount.accountId,
  //     approved_nft_ids: ['shardsquares.factory.defishards.near'],
  //     current_fee: 100
  //   }
  // })
  await ownerAccount.functionCall({
    contractId: marketplace.contractId,
    methodName: "add_approved_nft_contract_ids", 
    args: {
      nft_contract_ids: ['shardsquares.factory.defishards.near'],
    },
    gas,
    attachedDeposit: "1"
  })
}

async function view() {
 const config = await ownerAccount.viewFunction(
  marketplace.contractId,
  "get_config",
  {}
 )
 console.log('config: ', config)
 const approved_nft_contract_ids = await ownerAccount.viewFunction(
  marketplace.contractId,
  "approved_nft_contract_ids",
  {}
 )
 console.log('approved_nft_contract_ids: ', approved_nft_contract_ids)
 const marketData = await ownerAccount.viewFunction(
  marketplace.contractId,
  "get_market_data",
  {
    nft_contract_id: "uss-3.master.vierds.testnet",
    token_id: "5"
  }
 )
 console.log('marketData: ', marketData)
}
init()
// view()
