import { getContract, type PublicClient, type WalletClient } from 'viem';

import HubArtifact from '../assets/abi/Hub.json' assert { type: 'json' };
import MinterArtifact from '../assets/abi/Minter.json' assert { type: 'json' };
import VerifierArtifact from '../assets/abi/Verifier.json' assert { type: 'json' };
import Zerc20Artifact from '../assets/abi/zERC20.json' assert { type: 'json' };

import { normalizeHex } from '../utils/hex.js';

type ContractClient = PublicClient | WalletClient;

function toAddress(address: string): `0x${string}` {
  return normalizeHex(address) as `0x${string}`;
}

export function getZerc20Contract(address: string, client: ContractClient) {
  return getContract({
    address: toAddress(address),
    abi: Zerc20Artifact.abi,
    client,
  });
}

export function getMinterContract(address: string, client: ContractClient) {
  return getContract({
    address: toAddress(address),
    abi: MinterArtifact.abi,
    client,
  });
}

export function getVerifierContract(address: string, client: ContractClient) {
  return getContract({
    address: toAddress(address),
    abi: VerifierArtifact.abi,
    client,
  });
}

export function getHubContract(address: string, client: ContractClient) {
  return getContract({
    address: toAddress(address),
    abi: HubArtifact.abi,
    client,
  });
}

export { Zerc20Artifact, VerifierArtifact, HubArtifact, MinterArtifact };
