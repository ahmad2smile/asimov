import { Bot } from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { Card, CardContent } from '@/components/ui/card';
import { humanize, type Agv } from '@/lib/fleet';
import { ActivityLabel, ConnectionIcon, connectionLabel } from './indicators';

export function AgvDetail({ agv }: { agv: Agv | undefined }) {
  if (!agv) {
    return (
      <Card className="h-full items-center justify-center text-sm text-muted-foreground">
        Select an AGV
      </Card>
    );
  }
  const { state } = agv;

  return (
    <Card className="h-full gap-0 overflow-hidden pt-0">
      <div className="flex h-40 items-center justify-center bg-[#173d57]">
        <Bot className="size-24 text-primary" strokeWidth={1.25} />
      </div>
      <CardContent className="space-y-5 pt-5">
        <div>
          <div className="text-lg font-medium">{agv.serialNumber}</div>
          <div className="text-sm text-muted-foreground">
            {agv.manufacturer} · {agv.siteName}
          </div>
        </div>

        <div className="flex flex-wrap gap-2">
          <Badge variant="secondary" className="gap-1.5">
            <ConnectionIcon connection={agv.connection} className="size-3.5" />
            {connectionLabel(agv.connection)}
          </Badge>
          <Badge variant="secondary">
            <ActivityLabel activity={agv.activity} />
          </Badge>
        </div>

        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-2 text-sm">
          <Field label="Order" value={state.orderId || '—'} />
          <Field label="Last node" value={state.lastNodeId || '—'} />
          <Field
            label="Alerts"
            value={
              state.error ? `${humanize(state.error.errorType)} (${state.error.errorLevel.tag.toLowerCase()})` : '—'
            }
          />
        </dl>
      </CardContent>
    </Card>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="truncate" title={value}>
        {value}
      </dd>
    </>
  );
}
