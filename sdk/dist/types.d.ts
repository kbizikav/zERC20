export interface SecretAndTweak {
    secret: string;
    tweak: string;
}
export interface GeneralRecipient {
    chainId: bigint;
    address: string;
    tweak: string;
    fr: string;
    u256: string;
}
export interface BurnArtifacts {
    burnAddress: string;
    fullBurnAddress: string;
    secret: string;
    tweak: string;
    generalRecipient: GeneralRecipient;
}
export interface AggregationTreeState {
    latestAggSeq: bigint;
    aggregationRoot: string;
    snapshot: string[];
    transferTreeIndices: bigint[];
    chainIds: bigint[];
}
export interface GlobalTeleportProof {
    siblings: string[];
    leafIndex: bigint;
}
export interface IndexedEvent {
    eventIndex: bigint;
    from: string;
    to: string;
    value: bigint;
    ethBlockNumber: bigint;
}
export interface SingleTeleportArtifacts {
    proofCalldata: string;
    publicInputs: string[];
    treeDepth: number;
}
export interface SingleTeleportParams {
    wasmArtifacts: {
        localPk: Uint8Array;
        localVk: Uint8Array;
        globalPk: Uint8Array;
        globalVk: Uint8Array;
    };
    aggregationState: AggregationTreeState;
    recipientFr: string;
    secretHex: string;
    event: IndexedEvent;
    proof: GlobalTeleportProof;
}
export interface NovaProverInput {
    wasmArtifacts: {
        localPp: Uint8Array;
        localVp: Uint8Array;
        globalPp: Uint8Array;
        globalVp: Uint8Array;
    };
    aggregationState: AggregationTreeState;
    recipientFr: string;
    secretHex: string;
    proofs: readonly GlobalTeleportProof[];
    events: readonly IndexedEvent[];
}
export interface NovaProverOutput {
    ivcProof: Uint8Array;
    finalState: string[];
    steps: number;
}
//# sourceMappingURL=types.d.ts.map