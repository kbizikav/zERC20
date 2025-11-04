import type { ReactNode } from 'react';

interface DetailMetadataItem {
  label: string;
  value: ReactNode;
}

export interface RedeemDetailItem {
  id: string;
  title: string;
  burnAddress: string;
  secret: string;
  tweak: string;
  eligibleCount: number;
  totalEvents: number;
  onRedeem: () => void;
  redeemDisabled?: boolean;
  redeemedLabel?: string;
  eligibleValue?: string;
  pendingValue?: string;
}

interface RedeemDetailSectionProps {
  title: string;
  metadata: DetailMetadataItem[];
  items: RedeemDetailItem[];
  message?: string;
}

export function RedeemDetailSection({
  title,
  metadata,
  items,
  message,
}: RedeemDetailSectionProps): JSX.Element {
  return (
    <div className="card-section">
      <h3>{title}</h3>
      {metadata.length > 0 && (
        <ul className="detail-metadata">
          {metadata.map((entry) => (
            <li key={entry.label}>
              <span className="detail-label">{entry.label}</span>
              <span className="detail-value">{entry.value}</span>
            </li>
          ))}
        </ul>
      )}
      <div className="card-body">
        {items.map((item) => (
          <div key={item.id} className="burn-card">
            <header>
              <strong>{item.title}</strong>
            </header>
            <div className="detail-row">
              <span className="detail-label">Eligible events</span>
              <span className="detail-value">
                {item.eligibleCount} / {item.totalEvents}
              </span>
            </div>
            <footer>
              {item.redeemedLabel ? (
                <span className="info">{item.redeemedLabel}</span>
              ) : (
                <button type="button" onClick={item.onRedeem} disabled={item.redeemDisabled}>
                  Redeem
                </button>
              )}
            </footer>
          </div>
        ))}
      </div>
      {message && <p className="info">{message}</p>}
    </div>
  );
}
