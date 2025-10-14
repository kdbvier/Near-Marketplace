const nearAPI = require("near-api-js");
const {
  utils: {
    format: { parseNearAmount },
  },
} = nearAPI;
const getConfig = require("./config");
const {
  gas,
  gas_max,
  nodeUrl,
  walletUrl,
  ownerAccountName,
  contractName,
  networkId,
  marketplaceContract
} = getConfig("mainnet");

const keyStore1 = new nearAPI.keyStores.InMemoryKeyStore();
const PRIVATE_KEY1 =
  "ed25519:4fko4CFSmteMw2U4HUzSuBQNfdB91fVPFS8KDfxA8gaLE6tBk8Y5JT5FAXJFUfrs4czUug2eVCbWgXhKYhAiSZoh"; //defishards
const keyPair1 = nearAPI.KeyPair.fromString(PRIVATE_KEY1);

keyStore1.setKey("mainnet", "defishards.near", keyPair1);

const near1 = new nearAPI.Near({
  deps: {
    keyStore: keyStore1,
  },
  networkId: networkId,
  keyStore: keyStore1,
  nodeUrl: nodeUrl,
  walletUrl: walletUrl,
});

const ownerAccount = new nearAPI.Account(near1.connection, ownerAccountName);

const keyStore2 = new nearAPI.keyStores.InMemoryKeyStore();
const PRIVATE_KEY2 =
  "ed25519:31Hvsifgw7kpN55sa3N6F8L6tA4Ge7XUaXtcc7nCCxTNA2MvBciR7MgC4fhypCyLN9PCnvPZtDA7UgizufCY6qNU"; //viernear
const keyPair2 = nearAPI.KeyPair.fromString(PRIVATE_KEY2);

keyStore2.setKey("testnet", "viernear.testnet", keyPair2);

const near2 = new nearAPI.Near({
  deps: {
    keyStore: keyStore2,
  },
  networkId: networkId,
  keyStore: keyStore2,
  nodeUrl: nodeUrl,
  walletUrl: walletUrl,
});

const userAccount = new nearAPI.Account(near2.connection, "viernear.testnet");

const mainContract = new nearAPI.Contract(ownerAccount, contractName, {
  viewMethods: [],
  changeMethods: [],
});
const marketplace = new nearAPI.Contract(ownerAccount, marketplaceContract, {
  viewMethods: [],
  changeMethods: [],
})

module.exports = {
  gas,
  gas_max,
  ownerAccount,
  mainContract,
  userAccount,
  marketplace
};
