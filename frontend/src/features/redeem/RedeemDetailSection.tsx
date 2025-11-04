import type { ReactNode } from 'react';

interface DetailMetadataItem {
  label: string;
  value: ReactNode;
}

export interface RedeemDetailChainSummary {
  chainId: string;
  name: string;
  eligibleValue: string;
  pendingValue: string;
  eligibleEvents: ReadonlyArray<{
    eventIndex: string;
    from: string;
    to: string;
    value: string;
  }>;
  pendingEvents: ReadonlyArray<{
    eventIndex: string;
    from: string;
    to: string;
    value: string;
  }>;
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
  chainSummaries?: RedeemDetailChainSummary[];
}

interface RedeemDetailSectionProps {
  title: string;
  metadata: DetailMetadataItem[];
  items: RedeemDetailItem[];
  message?: ReactNode;
  onReload?: () => void;
  reloadDisabled?: boolean;
  isReloading?: boolean;
}

export function RedeemDetailSection({
  title,
  metadata,
  items,
  message,
  onReload,
  reloadDisabled = false,
  isReloading = false,
}: RedeemDetailSectionProps): JSX.Element {
  const showReload = Boolean(onReload);
  const reloadButtonClass = isReloading ? 'detail-reload-button loading' : 'detail-reload-button';

  return (
    <div className="card-section">
      <h3 className={showReload ? 'detail-section-title' : undefined}>
        <span>{title}</span>
        {showReload && (
          <button
            type="button"
            className={reloadButtonClass}
            onClick={onReload}
            disabled={reloadDisabled}
            aria-label="Reload detail"
            title="Reload detail"
          >
            <span aria-hidden="true" className="detail-reload-icon">
              ↻
            </span>
          </button>
        )}
      </h3>
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
              <code className="mono">{item.burnAddress}</code>
              {item.title && <span className="badge">{item.title}</span>}
            </header>
            <div className="detail-row">
              <span className="detail-label">Eligible events</span>
              <span className="detail-value">
                {item.eligibleCount} / {item.totalEvents}
              </span>
            </div>
            {item.eligibleValue && (
              <div className="detail-row">
                <span className="detail-label">Eligible value</span>
                <span className="detail-value">
                  <code className="mono">{item.eligibleValue}</code>
                </span>
              </div>
            )}
            {item.pendingValue && (
              <div className="detail-row">
                <span className="detail-label">Pending value</span>
                <span className="detail-value">
                  <code className="mono">{item.pendingValue}</code>
                </span>
              </div>
            )}
            {item.chainSummaries?.map((chain) => (
              <div key={`${item.id}-${chain.chainId}`} className="chain-summary">
                <div className="chain-summary-header">
                  <span className="chain-name">{chain.name}</span>
                  <span className="chain-id">Chain ID {chain.chainId}</span>
                </div>
                <div className="chain-summary-stats">
                  <span>
                    Eligible <code className="mono">{chain.eligibleValue}</code>
                  </span>
                  <span>
                    Pending <code className="mono">{chain.pendingValue}</code>
                  </span>
                </div>
                {chain.eligibleEvents.length > 0 && (
                  <div className="chain-events-group">
                    <div className="chain-events-header">
                      <span className="detail-label">Eligible transfers</span>
                      <span className="detail-value">{chain.eligibleEvents.length}</span>
                    </div>
                    <ul className="events eligible">
                      {chain.eligibleEvents.map((event) => (
                        <li
                          key={`eligible-${event.eventIndex}-${event.from}-${event.to}`}
                          className="event-line"
                        >
                          <span aria-hidden="true" className="event-icon">
                            ✅
                          </span>
                          <div className="event-details">
                            <span className="event-meta">
                              Index {event.eventIndex} · Value{' '}
                              <code className="mono">{event.value}</code>
                            </span>
                            <span className="event-address">
                              <span>
                                From <code className="mono">{event.from}</code>
                              </span>
                              <span>
                                To <code className="mono">{event.to}</code>
                              </span>
                            </span>
                          </div>
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
                {chain.pendingEvents.length > 0 && (
                  <div className="chain-events-group">
                    <div className="chain-events-header">
                      <span className="detail-label">Pending transfers</span>
                      <span className="detail-value">{chain.pendingEvents.length}</span>
                    </div>
                    <ul className="events pending">
                      {chain.pendingEvents.map((event) => (
                        <li
                          key={`pending-${event.eventIndex}-${event.from}-${event.to}`}
                          className="event-line"
                        >
                          <span aria-hidden="true" className="event-icon">
                            ⏳
                          </span>
                          <div className="event-details">
                            <span className="event-meta">
                              Index {event.eventIndex} · Value{' '}
                              <code className="mono">{event.value}</code>
                            </span>
                            <span className="event-address">
                              <span>
                                From <code className="mono">{event.from}</code>
                              </span>
                              <span>
                                To <code className="mono">{event.to}</code>
                              </span>
                            </span>
                          </div>
                        </li>
                      ))}
                    </ul>
                  </div>
                )}
              </div>
            ))}
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
