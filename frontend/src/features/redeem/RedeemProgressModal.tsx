import type { RedeemStep } from './redeemSteps';

interface RedeemProgressModalProps {
  open: boolean;
  steps: RedeemStep[];
  onClose: () => void;
  title?: string;
  message?: string;
}

function statusLabel(status: RedeemStep['status']): string {
  switch (status) {
    case 'active':
      return 'In progress';
    case 'done':
      return 'Done';
    case 'error':
      return 'Failed';
    case 'pending':
    default:
      return 'Pending';
  }
}

export function RedeemProgressModal({
  open,
  steps,
  onClose,
  title = 'Redeem progress',
  message,
}: RedeemProgressModalProps): JSX.Element | null {
  if (!open) {
    return null;
  }

  const isProcessing = steps.some((step) => step.status === 'active');

  return (
    <div className="modal-overlay" role="dialog" aria-modal="true" aria-label={title}>
      <div className="modal">
        <header className="modal-header">
          <h3>{title}</h3>
          <button type="button" className="compact" onClick={onClose} disabled={isProcessing}>
            Close
          </button>
        </header>
        <div className="modal-body">
          <ul className="redeem-steps">
            {steps.map((step) => (
              <li key={step.id} className={`redeem-step ${step.status}`}>
                <span className="label">{step.label}</span>
                <span className="status">{statusLabel(step.status)}</span>
              </li>
            ))}
          </ul>
          {message && <p className="modal-message">{message}</p>}
        </div>
      </div>
    </div>
  );
}
