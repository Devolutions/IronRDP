import {Component} from "@angular/core";
import {Session} from "../../models/session";
import {SessionService} from "../services/session.service";

@Component({
  selector: 'app-session',
  templateUrl: 'session.component.html',
  styleUrls: ['session.component.scss']
})
export class SessionComponent {
  currentSession: Session;
  connected = false;

  constructor(public sessionService: SessionService) {
    this.sessionService.currentSession$.subscribe(session => this.currentSession = session);
  }
}
