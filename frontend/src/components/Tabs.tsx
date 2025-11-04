import { ReactNode } from 'react';

export interface TabItem {
  id: string;
  label: string;
  disabled?: boolean;
}

interface TabsProps {
  items: TabItem[];
  activeId: string;
  onSelect: (id: string) => void;
  renderLabel?: (item: TabItem) => ReactNode;
  ariaLabel?: string;
}

export function Tabs({ items, activeId, onSelect, renderLabel, ariaLabel = 'Section navigation' }: TabsProps): JSX.Element {
  return (
    <div className="tabs" role="tablist" aria-label={ariaLabel} aria-orientation="horizontal">
      {items.map((item) => {
        const isActive = item.id === activeId;
        const tabId = `tab-${item.id}`;
        const panelId = `panel-${item.id}`;
        return (
          <button
            key={item.id}
            type="button"
            role="tab"
            id={tabId}
            aria-controls={panelId}
            aria-selected={isActive}
            className={isActive ? 'tab active' : 'tab'}
            disabled={item.disabled}
            tabIndex={item.disabled ? -1 : isActive ? 0 : -1}
            onClick={() => onSelect(item.id)}
          >
            <span className="tab-label">{renderLabel ? renderLabel(item) : item.label}</span>
          </button>
        );
      })}
    </div>
  );
}
