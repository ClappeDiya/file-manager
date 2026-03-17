'use client';

import React, { useState } from 'react';
import { cn } from '@/lib/utils/cn';

interface TabsProps {
  tabs: { id: string; label: string; count?: number }[];
  activeTab?: string;
  onTabChange?: (tabId: string) => void;
  children?: React.ReactNode;
  className?: string;
}

export function Tabs({ tabs, activeTab, onTabChange, children, className }: TabsProps) {
  const [active, setActive] = useState(activeTab || tabs[0]?.id);

  const handleTabChange = (tabId: string) => {
    setActive(tabId);
    onTabChange?.(tabId);
  };

  return (
    <div className={className}>
      <div className="flex border-b border-border">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            onClick={() => handleTabChange(tab.id)}
            className={cn(
              'px-4 py-2.5 text-sm font-medium transition-colors relative',
              active === tab.id
                ? 'text-primary border-b-2 border-primary -mb-px'
                : 'text-foreground-secondary hover:text-foreground hover:bg-background-secondary'
            )}
          >
            {tab.label}
            {tab.count !== undefined && (
              <span className={cn(
                'ml-2 inline-flex items-center justify-center min-w-[1.25rem] h-5 rounded-full text-xs px-1.5',
                active === tab.id
                  ? 'bg-primary text-primary-foreground'
                  : 'bg-background-tertiary text-foreground-secondary'
              )}>
                {tab.count}
              </span>
            )}
          </button>
        ))}
      </div>
      {children}
    </div>
  );
}

interface TabContentProps {
  tabId: string;
  activeTab: string;
  children: React.ReactNode;
  className?: string;
}

export function TabContent({ tabId, activeTab, children, className }: TabContentProps) {
  if (tabId !== activeTab) return null;
  return <div className={cn('pt-4', className)}>{children}</div>;
}
