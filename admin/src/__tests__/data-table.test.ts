/** DataTable renders the default columns (hidden ones only when chosen) and the pager from `pagination`. */
import { describe, expect, it } from 'vitest';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { DataTable, type Column } from '@/components/ui/data-table';

type Row = { id: string; name: string; secret: string };
const columns: Column<Row>[] = [
  { key: 'id', label: 'Id', required: true, render: (r) => r.id },
  { key: 'name', label: 'Name', render: (r) => r.name },
  { key: 'secret', label: 'Hidden column', defaultHidden: true, render: (r) => r.secret },
];

describe('DataTable', () => {
  it('shows default columns, hides opt-in ones and labels the page', () => {
    const html = renderToStaticMarkup(
      React.createElement(DataTable<Row>, {
        id: 'test',
        columns,
        rows: [{ id: 'r1', name: 'Alpha', secret: 'only-when-chosen' }],
        rowKey: (r: Row) => r.id,
        pagination: { limit: 25, offset: 25, total: 51, nextCursor: 'x', hasMore: true },
        limit: 25,
        onOffset: () => undefined,
        onLimit: () => undefined,
      }),
    );
    expect(html).toContain('Alpha');
    expect(html).toContain('>Name<');
    expect(html).not.toContain('only-when-chosen');
    expect(html).not.toContain('>Hidden column<');
    expect(html).toContain('26–26 of 51');
  });
});
