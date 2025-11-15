import {
  createTeleportProofClient,
  type BatchTeleportArtifacts,
  type BatchTeleportParams,
  type TeleportProofClient,
  type SingleTeleportArtifacts,
  type SingleTeleportParams,
} from '@zerc20/sdk';

let teleportClient: TeleportProofClient | undefined;

function getTeleportProofClient(): TeleportProofClient {
  if (!teleportClient) {
    teleportClient = createTeleportProofClient();
  }
  return teleportClient;
}

export async function generateSingleTeleportProof(
  params: SingleTeleportParams,
): Promise<SingleTeleportArtifacts> {
  return getTeleportProofClient().createSingleTeleportProof(params);
}

export async function generateBatchTeleportProof(
  params: BatchTeleportParams,
): Promise<BatchTeleportArtifacts> {
  return getTeleportProofClient().createBatchTeleportProof(params);
}
