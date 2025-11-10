import { Contract, ContractRunner, Interface } from 'ethers';

import HubArtifact from '../assets/abi/Hub.json' assert { type: 'json' };
import MinterArtifact from '../assets/abi/Minter.json' assert { type: 'json' };
import VerifierArtifact from '../assets/abi/Verifier.json' assert { type: 'json' };
import Zerc20Artifact from '../assets/abi/zERC20.json' assert { type: 'json' };

const ZERC20_INTERFACE = new Interface(Zerc20Artifact.abi);
const VERIFIER_INTERFACE = new Interface(VerifierArtifact.abi);
const MINTER_INTERFACE = new Interface(MinterArtifact.abi);

export function getZerc20Contract(address: string, runner: ContractRunner): Contract {
  return new Contract(address, ZERC20_INTERFACE, runner);
}

export function getMinterContract(address: string, runner: ContractRunner): Contract {
  return new Contract(address, MINTER_INTERFACE, runner);
}

export function getVerifierContract(address: string, runner: ContractRunner): Contract {
  return new Contract(address, VERIFIER_INTERFACE, runner);
}

export { Zerc20Artifact, VerifierArtifact, HubArtifact, MinterArtifact };
