import type { ReactNode } from 'react';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { cn } from '@/lib/utils';
import { humanize, type Agv } from '@/lib/fleet';
import { ActivityLabel, connectionLabel } from './indicators';

interface AgvTableProps {
  agvs: Agv[];
  selectedId: string | undefined;
  onSelect: (id: string) => void;
  /** Rendered under the table, e.g. a `Pager`. */
  footer?: ReactNode;
}

export function AgvTable({ agvs, selectedId, onSelect, footer }: AgvTableProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Fleet</CardTitle>
      </CardHeader>
      <CardContent>
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead>AGV</TableHead>
              <TableHead>Site</TableHead>
              <TableHead>Activity</TableHead>
              <TableHead>Order</TableHead>
              <TableHead>Alerts</TableHead>
              <TableHead className="text-right">Updated</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {agvs.length === 0 && (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={6} className="py-8 text-center text-muted-foreground">
                  No AGVs match.
                </TableCell>
              </TableRow>
            )}
            {agvs.map(agv => (
              <TableRow
                key={agv.id}
                onClick={() => onSelect(agv.id)}
                data-state={agv.id === selectedId ? 'selected' : undefined}
                className="cursor-pointer"
              >
                <TableCell>
                  <div className="flex items-center gap-2" title={connectionLabel(agv.connection)}>
                    <span className="font-medium">{agv.serialNumber}</span>
                    <span className="text-muted-foreground">{agv.manufacturer}</span>
                  </div>
                </TableCell>
                <TableCell>{agv.siteName}</TableCell>
                <TableCell>
                  <ActivityLabel activity={agv.activity} />
                </TableCell>
                <TableCell className="max-w-48 truncate text-muted-foreground">
                  {agv.state.orderId || '—'}
                </TableCell>
                <TableCell className={cn(agv.state.error ? 'text-warning' : 'text-muted-foreground')}>
                  {agv.state.error ? humanize(agv.state.error.errorType) : '—'}
                </TableCell>
                <TableCell className="text-right tabular-nums text-muted-foreground">
                  {agv.state.updatedAt.toDate().toLocaleTimeString()}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
        {footer}
      </CardContent>
    </Card>
  );
}
