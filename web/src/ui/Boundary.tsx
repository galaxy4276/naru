import { Component, type ReactNode } from 'react';

interface Props {
  /** The region, for the message: "{name}에서 오류가 났어요". */
  name: string;
  children: ReactNode;
  className?: string;
}

/**
 * Keeps a rendering error inside one region: the rest of the window and the agents behind it
 * keep running, and the region says what broke instead of going blank.
 */
export class Boundary extends Component<Props, { error: Error | null }> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error) {
    console.error(`${this.props.name} 그리기 오류:`, error);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div role="alert" className={this.props.className ?? 'm-2 rounded-xl bg-background p-3 text-[12.5px]'}>
        <b className="font-semibold">{this.props.name}에서 오류가 났어요</b>
        <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{error.message}</p>
        <button onClick={() => this.setState({ error: null })} className="mt-2 rounded-md bg-muted px-2 py-1 font-medium hover:bg-accent">
          다시 그리기
        </button>
      </div>
    );
  }
}
