export type RedeemStage = 'indexer' | 'proof' | 'decider' | 'wallet';

export type RedeemStepStatus = 'pending' | 'active' | 'done' | 'error';

export interface RedeemStep {
  id: RedeemStage;
  label: string;
  status: RedeemStepStatus;
}

export function createRedeemSteps(isBatch: boolean): RedeemStep[] {
  const base: RedeemStep[] = [
    { id: 'indexer', label: 'Query indexer', status: 'pending' },
    { id: 'proof', label: 'Generate WASM proof', status: 'pending' },
  ];
  if (isBatch) {
    base.push({ id: 'decider', label: 'Request decider', status: 'pending' });
  }
  base.push({ id: 'wallet', label: 'Submit wallet tx', status: 'pending' });
  return base;
}

export function setStepStatus(steps: RedeemStep[], id: RedeemStage, status: RedeemStepStatus): RedeemStep[] {
  return steps.map((step) => (step.id === id ? { ...step, status } : step));
}
