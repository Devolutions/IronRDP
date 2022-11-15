import {Injectable} from "@angular/core";
import {BehaviorSubject, Observable, Subject} from "rxjs";
import {invoke} from "@tauri-apps/api";
import {environment} from "../environments/environment";

@Injectable({
  providedIn: 'root'
})
export class ApplicationService {
  splashScreenReady$: Observable<boolean>;
  splashScreenReady: BehaviorSubject<boolean> = new BehaviorSubject<boolean>(false);

  constructor() {
    this.splashScreenReady$ = this.splashScreenReady.asObservable()
  }

  setSplashScreenReady(value: boolean) {
    console.log("set splash ready to ", value);
    this.splashScreenReady.next(value);
  }

  closeSplashscreen() {
    if (environment.tauri) {
      invoke('close_splashscreen');
    }
  }
}
