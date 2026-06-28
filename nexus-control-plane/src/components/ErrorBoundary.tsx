import { Component, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  error: Error | null;
}

export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[NCC] React error boundary caught:", error, info.componentStack);
  }

  render() {
    if (!this.state.error) {
      return this.props.children;
    }

    return (
      <div className="flex flex-col items-center justify-center h-full bg-nx-bg text-nx-text px-8">
        <div className="max-w-md text-center">
          <div className="text-4xl mb-4">&#x26A0;</div>
          <h1 className="text-lg font-heading font-semibold mb-2">
            Something went wrong
          </h1>
          <p className="text-sm font-body text-nx-text-secondary mb-4">
            NCC hit an unexpected error. Your sessions are still running in the background.
          </p>
          <pre className="text-[11px] font-mono text-nx-muted bg-nx-surface border border-nx-border rounded-lg p-3 mb-6 text-left overflow-x-auto max-h-32 overflow-y-auto">
            {this.state.error.message}
          </pre>
          <div className="flex gap-3 justify-center">
            <button
              onClick={() => this.setState({ error: null })}
              className="px-4 py-2 bg-nx-accent text-white rounded-lg font-body text-sm font-medium hover:bg-nx-accent-hover transition-colors"
            >
              Try Again
            </button>
            <button
              onClick={() => window.location.reload()}
              className="px-4 py-2 bg-nx-surface border border-nx-border rounded-lg font-body text-sm text-nx-text hover:bg-nx-surface-hover transition-colors"
            >
              Reload App
            </button>
          </div>
        </div>
      </div>
    );
  }
}
