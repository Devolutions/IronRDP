import {AfterViewInit, Component} from "@angular/core";
import {ApplicationService} from "../application.service";

@Component({
  selector: 'app-splashscreen',
  templateUrl: 'splashscreen.component.html',
  styleUrls: ['splashscreen.component.scss']
})
export class SplashscreenComponent implements AfterViewInit {

  constructor(private applicationService: ApplicationService) {
  }

  ngAfterViewInit() {
    this.applicationService.setSplashScreenReady(true);
  }
}
