export class LoggingService {
    verbose: boolean = false;
    
    info(description: string) {
        if (this.verbose) {
            console.log(description);   
        }
    }
    
    error(description: string, object?: any) {
        if (this.verbose) {
            console.error(description, object);
        }
    }
}

export let loggingService = new LoggingService();