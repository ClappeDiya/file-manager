import { Check, X, Minus } from 'lucide-react';

type Cell = '✓' | '✗' | '~' | string;

const rows: { label: string; cells: [Cell, Cell, Cell, Cell] }[] = [
  { label: 'Free for individuals', cells: ['✓', '✗', '✗', '✓'] },
  { label: 'macOS / Windows / Linux', cells: ['✓', 'macOS only', 'macOS only', '✓'] },
  { label: 'Built-in protocols', cells: ['17', '11', '8', '~10'] },
  { label: 'Crash-safe resume', cells: ['✓', '~', '~', '✗'] },
  { label: 'Real-time bidirectional sync', cells: ['✓', '✗', '✗', '~'] },
  { label: 'Built-in local AI', cells: ['✓', '✗', '✗', '✗'] },
  { label: 'Encryption + vault', cells: ['✓', '✗', '~', '~'] },
  { label: 'Team governance (audit / SSO)', cells: ['✓ Business', '✗', '✗', '✗'] },
  { label: 'Source-available', cells: ['✓', '✗', '✗', '✓'] },
];

const cols = ['FileManager', 'ForkLift', 'Transmit', 'Cyberduck'] as const;

function renderCell(value: Cell, isFileManager: boolean) {
  if (value === '✓') return <Check size={18} className={isFileManager ? 'text-success' : 'text-foreground-secondary'} />;
  if (value === '✗') return <X size={18} className="text-foreground-tertiary" />;
  if (value === '~') return <Minus size={18} className="text-warning" />;
  return <span className={`text-xs ${isFileManager ? 'text-success' : 'text-foreground-secondary'}`}>{value}</span>;
}

export function ComparisonTable() {
  return (
    <section className="marketing-section">
      <div className="container-page">
        <div className="text-center max-w-3xl mx-auto mb-10">
          <h2 className="marketing-heading">How it compares</h2>
          <p className="marketing-subheading mt-4">
            We checked, you can verify. Marketing claims of competing products as of {new Date().getFullYear()}.
          </p>
        </div>

        <div className="overflow-x-auto rounded-lg border border-border bg-background-elevated">
          <table className="w-full text-sm">
            <thead className="bg-background-secondary">
              <tr>
                <th className="text-left px-4 py-3 font-medium text-foreground-secondary">Feature</th>
                {cols.map((col, idx) => (
                  <th
                    key={col}
                    className={`text-center px-4 py-3 font-semibold ${idx === 0 ? 'text-primary' : 'text-foreground-secondary'}`}
                  >
                    {col}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((row, idx) => (
                <tr key={row.label} className={idx % 2 === 0 ? '' : 'bg-background-secondary/40'}>
                  <td className="px-4 py-3 text-foreground">{row.label}</td>
                  {row.cells.map((cell, cellIdx) => (
                    <td key={cellIdx} className="px-4 py-3">
                      <div className="flex items-center justify-center">{renderCell(cell, cellIdx === 0)}</div>
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </section>
  );
}
