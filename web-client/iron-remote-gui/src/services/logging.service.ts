export class LoggingService {
    verbose: boolean = false;
    
    info(description: string) {
        if (this.verbose) {
            console.log(description);   
        }
    }
    
    error(description: string, object?: unknown) {
        if (this.verbose) {
            console.error(description, object);
        }
    }
}

export const loggingService = new LoggingService();