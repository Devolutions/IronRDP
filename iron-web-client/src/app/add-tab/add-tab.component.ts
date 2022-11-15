import { Component, OnInit } from '@angular/core';
import {SessionService} from "../services/session.service";

@Component({
  selector: 'app-add-tab',
  templateUrl: './add-tab.component.html',
  styleUrls: ['./add-tab.component.scss']
})
export class AddTabComponent {

  constructor(public sessionService: SessionService) { }

  addSession(){
    this.sessionService.addSession('New Session')
  }
}
